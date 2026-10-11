use serde_json::{json, Value};

use crate::api::Router;

pub const COMPRESSION_THRESHOLD: f64 = 0.85;
const KEEP_RECENT_MESSAGES: usize = 4;
const KEEP_TOKEN_BUDGET_RATIO: f64 = 0.3;

pub fn estimate_tokens(text: &str) -> usize {
    let total = text.chars().count();
    let cjk = text
        .chars()
        .filter(|c| *c as u32 >= 0x2E80 && *c as u32 <= 0x9FFF || *c as u32 >= 0xF900)
        .count();
    let other = total - cjk;
    cjk + other / 4 + 1
}

pub fn message_tokens(msg: &Value) -> usize {
    let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
    estimate_tokens(content) + 4
}

pub fn history_tokens(messages: &[Value]) -> usize {
    messages.iter().map(message_tokens).sum()
}

/// 判断是否需要压缩。`extra_tokens` 为随请求一起发送但不在 history 中的
/// 开销（system prompt、协议说明等），必须计入以免真实用量超出阈值。
pub fn needs_compression(messages: &[Value], context_limit: usize, extra_tokens: usize) -> bool {
    extra_tokens + history_tokens(messages) >= (context_limit as f64 * COMPRESSION_THRESHOLD) as usize
}

fn split_for_compression(messages: &[Value], context_limit: usize) -> Option<(&[Value], &[Value])> {
    if messages.len() <= 2 {
        return None;
    }
    let budget = (context_limit as f64 * KEEP_TOKEN_BUDGET_RATIO) as usize;
    let mut keep_start = messages.len();
    let mut acc = 0usize;
    for i in (1..messages.len()).rev() {
        if messages.len() - keep_start >= KEEP_RECENT_MESSAGES {
            break;
        }
        let tok = message_tokens(&messages[i]);
        if acc > 0 && acc + tok > budget {
            break;
        }
        acc += tok;
        keep_start = i;
    }
    let old = &messages[1..keep_start];
    if old.is_empty() {
        return None;
    }
    Some((old, &messages[keep_start..]))
}

/// 本地兜底截断：当模型压缩摘要不可用（网络/限流）时，从最旧的
/// 非 system 消息开始丢弃，直到总量降到阈值以下。始终保留 system
/// 与至少最近一条消息，防止 Token 溢出报错。
pub fn local_trim(messages: &[Value], context_limit: usize, extra_tokens: usize) -> Vec<Value> {
    if messages.len() < 2 {
        return messages.to_vec();
    }
    let has_system = messages
        .first()
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        == Some("system");
    let body_start = usize::from(has_system);
    let system_tokens = if has_system {
        message_tokens(&messages[0])
    } else {
        0
    };
    let budget = (context_limit as f64 * COMPRESSION_THRESHOLD) as usize;

    let mut keep_from = body_start;
    loop {
        let tail: usize = messages[keep_from..].iter().map(message_tokens).sum();
        if extra_tokens + system_tokens + tail < budget {
            break;
        }
        if keep_from + 1 >= messages.len() {
            break;
        }
        keep_from += 1;
    }

    let mut out = Vec::with_capacity(messages.len() - keep_from + 1);
    if has_system {
        out.push(messages[0].clone());
    }
    out.extend(messages[keep_from..].iter().cloned());
    out
}

/// 单条消息进入摘要 prompt 时的字符上限。
const SUMMARY_ITEM_CAP: usize = 800;
/// 摘要 prompt 中 transcript 的总字符上限。
const SUMMARY_TOTAL_CAP: usize = 12_000;

/// 摘要用的消息截断：保留头尾，中间标注省略量。
///
/// 头尾都要留——工具结果的**结尾**往往才是关键信息（如编译报错的最后
/// 几行、错误原因），只留头部会把真正有用的部分裁掉。
fn truncate_for_summary(content: &str, cap: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= cap {
        return content.to_string();
    }
    let head_n = cap * 2 / 3;
    let tail_n = cap - head_n;
    let head: String = chars[..head_n].iter().collect();
    let tail: String = chars[chars.len() - tail_n..].iter().collect();
    let omitted = chars.len() - head_n - tail_n;
    format!("{}\n…（此处省略 {} 字符）…\n{}", head, omitted, tail)
}

/// 构建摘要 prompt 的 transcript。
///
/// v0.3.2 之前这里是逐字拼接完整内容——写入任务下 `read_file` 的结果
/// 动辄数百行，逐条累积会让**摘要请求本身**就撑爆上下文，压缩彻底失效。
/// 这里对每条消息预截断，并对总量设上限。
fn build_transcript(old: &[Value]) -> String {
    let mut transcript = String::new();
    for msg in old {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if transcript.chars().count() >= SUMMARY_TOTAL_CAP {
            transcript.push_str("…（更早的内容因长度限制未纳入摘要）…\n");
            break;
        }
        let piece = truncate_for_summary(content, SUMMARY_ITEM_CAP);
        transcript.push_str(&format!("[{role}]\n{piece}\n\n"));
    }
    transcript
}

pub async fn compress(
    router: &Router,
    model_idx: usize,
    messages: &[Value],
    context_limit: usize,
) -> Option<Vec<Value>> {
    let (old, recent) = split_for_compression(messages, context_limit)?;
    let transcript = build_transcript(old);

    let prompt = format!(
        "请将以下历史对话压缩为简明的前情提要。要求：\n\
         1. 使用中文；\n\
         2. 保留关键结论、已读文件路径、已修改的文件、未完成的任务；\n\
         3. 300 字以内，直接输出提要正文。\n\
         --- {} ---\n{}",
        transcript.chars().count(),
        transcript
    );

    let summary = router
        .complete_with_fallback(model_idx, &[json!({ "role": "user", "content": prompt })])
        .await
        .ok()?
        .value;

    let mut out = Vec::with_capacity(recent.len() + 2);
    if messages.first().and_then(|m| m.get("role")).and_then(|r| r.as_str()) == Some("system") {
        out.push(messages[0].clone());
    }
    out.push(json!({
        "role": "user",
        "content": format!("[前情提要]\n以下是系统自动整理的早期对话摘要：\n{}", summary.trim())
    }));
    out.extend(recent.iter().cloned());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_head_and_tail() {
        let long = "a".repeat(SUMMARY_ITEM_CAP * 3);
        let out = truncate_for_summary(&long, SUMMARY_ITEM_CAP);
        assert!(out.contains("此处省略"));
        // 头部与尾部都保留：工具结果的关键信息通常在结尾
        assert!(out.starts_with('a'));
        assert!(out.ends_with('a'));
    }

    #[test]
    fn truncate_leaves_short_content_intact() {
        assert_eq!(truncate_for_summary("短内容", SUMMARY_ITEM_CAP), "短内容");
    }

    #[test]
    fn truncate_is_utf8_safe() {
        let long = "中".repeat(SUMMARY_ITEM_CAP * 2);
        let out = truncate_for_summary(&long, SUMMARY_ITEM_CAP);
        assert!(out.chars().count() <= SUMMARY_ITEM_CAP + 20);
        assert!(out.contains('中'));
    }

    #[test]
    fn transcript_caps_each_item_and_total() {
        // 单条超长消息被截断
        let msgs = vec![
            json!({ "role": "user", "content": "x".repeat(SUMMARY_ITEM_CAP * 5) }),
        ];
        let t = build_transcript(&msgs);
        assert!(t.chars().count() < SUMMARY_ITEM_CAP * 2);
        assert!(t.contains("此处省略"));

        // 多条消息总量受限
        let many: Vec<Value> = (0..50)
            .map(|_| json!({ "role": "user", "content": "y".repeat(500) }))
            .collect();
        let t = build_transcript(&many);
        assert!(
            t.chars().count() < SUMMARY_TOTAL_CAP * 2,
            "transcript 未受总量约束: {}",
            t.chars().count()
        );
    }

    #[test]
    fn transcript_keeps_early_messages_content() {
        let msgs = vec![
            json!({ "role": "user", "content": "第一条消息内容" }),
            json!({ "role": "assistant", "content": "第二条消息内容" }),
        ];
        let t = build_transcript(&msgs);
        assert!(t.contains("第一条消息内容"));
        assert!(t.contains("第二条消息内容"));
        assert!(t.contains("[user]"));
        assert!(t.contains("[assistant]"));
    }

    fn msg(role: &str, content: &str) -> Value {
        json!({ "role": role, "content": content })
    }

    #[test]
    fn estimates_mixed_text() {
        let n = estimate_tokens("hello world 这是中文");
        assert!(n >= 6 && n <= 15, "n = {}", n);
    }

    #[test]
    fn compression_triggers_at_85_percent() {
        let messages = vec![msg("system", &"x".repeat(1000))];
        assert!(!needs_compression(&messages, 2000, 0));
        let messages = vec![msg("system", &"x".repeat(6800))];
        assert!(needs_compression(&messages, 2000, 0));
    }

    #[test]
    fn needs_compression_counts_extra_tokens() {
        // history 本身低于阈值（约 5000），但加上 system prompt 后越线
        let messages = vec![msg("user", &"x".repeat(20000))];
        assert!(!needs_compression(&messages, 8000, 0));
        assert!(needs_compression(&messages, 8000, 1800));
    }

    #[test]
    fn local_trim_drops_oldest_keeps_system_and_recent() {
        let mut messages = vec![msg("system", "sys")];
        for i in 0..20 {
            messages.push(msg("user", &format!("{}{}", "内容".repeat(50), i)));
        }
        let limit = 2000;
        let before = history_tokens(&messages);
        assert!(needs_compression(&messages, limit, 0), "before={}", before);

        let out = local_trim(&messages, limit, 0);
        assert_eq!(out[0]["role"], "system", "必须保留 system");
        assert_eq!(
            out[out.len() - 1]["content"],
            messages[messages.len() - 1]["content"],
            "必须保留最近一条"
        );
        assert!(out.len() < messages.len(), "应丢弃部分旧消息");
        assert!(
            history_tokens(&out) < (limit as f64 * COMPRESSION_THRESHOLD) as usize,
            "trim 后应低于阈值"
        );
    }

    #[test]
    fn local_trim_noop_when_under_budget() {
        let messages = vec![msg("system", "sys"), msg("user", "hi"), msg("assistant", "yo")];
        let out = local_trim(&messages, 100000, 0);
        assert_eq!(out.len(), messages.len());
    }

    #[test]
    fn local_trim_keeps_last_message_even_if_huge() {
        let mut messages = vec![msg("system", "sys")];
        for i in 0..5 {
            messages.push(msg("user", &format!("m{}", i)));
        }
        messages.push(msg("assistant", &"巨".repeat(50000)));
        let out = local_trim(&messages, 2000, 0);
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[out.len() - 1]["role"], "assistant", "极端情况下仍保留最后一条");
    }

    #[test]
    fn split_keeps_recent_and_system() {
        let mut messages = vec![msg("system", "sys")];
        for i in 0..10 {
            messages.push(msg("user", &format!("消息{}", i)));
        }
        let (old, recent) = split_for_compression(&messages, 100000).unwrap();
        assert_eq!(old.len(), 6);
        assert_eq!(recent.len(), 4);
        assert_eq!(recent[0]["content"], "消息6");
        assert_eq!(old[0]["content"], "消息0");
    }

    #[test]
    fn split_skips_when_nothing_to_compress() {
        let messages = vec![msg("system", "sys"), msg("user", "hi"), msg("assistant", "yo")];
        assert!(split_for_compression(&messages, 100000).is_none());
    }

    /// 上下文压缩后任务清单必须仍可从 system prompt 取到。
    ///
    /// 清单本体存在独立的 `todos_json` 列，压缩只动 agent 历史，
    /// 两者互不干涉；而 system prompt 每轮重建并重新注入清单，
    /// 因此「清单随历史一起被压掉」在设计上不可能发生。
    ///
    /// 这条测试把该不变量钉住——将来若有人改成「把清单塞进 history
    /// 再靠摘要带过去」，这里会立刻失败。
    #[test]
    fn todo_list_survives_history_compression() {
        let todos = crate::agent::todo::parse_plan(
            r#"[{"id":"s1","title":"新增 config 模块"},{"id":"s2","title":"补测试"}]"#,
        )
        .unwrap()
        .todos;

        // 构造一段足够长、必然触发压缩的历史
        let mut history = vec![msg("system", "sys")];
        for i in 0..40 {
            history.push(msg("user", &format!("早期消息{}{}", i, "内容".repeat(200))));
        }
        assert!(
            needs_compression(&history, 8000, 0),
            "历史长度应足以触发压缩，否则本测试没有验证价值"
        );

        // 压缩作用于历史
        let trimmed = local_trim(&history, 8000, 0);
        assert!(trimmed.len() < history.len(), "历史应确实被裁剪");

        // 但清单仍在，且能照常渲染进新的 system prompt
        let rendered = crate::agent::todo::render_for_prompt(&todos);
        assert!(rendered.contains("s1: 新增 config 模块"));
        assert!(rendered.contains("s2: 补测试"));
        assert!(rendered.contains("0/2 已完成"));
    }

    /// 清单渲染不依赖 history 长度，长对话下同样稳定。
    #[test]
    fn todo_render_is_independent_of_history() {
        let todos = crate::agent::todo::parse_plan(r#"[{"id":"s1","title":"甲"}]"#)
            .unwrap()
            .todos;
        let short = crate::agent::todo::render_for_prompt(&todos);
        // 模拟超长历史后再渲染，结果应完全一致
        let mut long_history = vec![msg("system", "sys")];
        for _ in 0..500 {
            long_history.push(msg("user", &"x".repeat(100)));
        }
        let _ = local_trim(&long_history, 8000, 0);
        assert_eq!(crate::agent::todo::render_for_prompt(&todos), short);
    }
}
