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

pub async fn compress(
    router: &Router,
    model_idx: usize,
    messages: &[Value],
    context_limit: usize,
) -> Option<Vec<Value>> {
    let (old, recent) = split_for_compression(messages, context_limit)?;

    let mut transcript = String::new();
    for msg in old {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
        transcript.push_str(&format!("[{}]\n{}\n\n", role, content));
    }

    let prompt = format!(
        "请将以下历史对话压缩为简明的前情提要。要求：\n\
         1. 使用中文；\n\
         2. 保留关键结论、已读文件路径、未完成的任务；\n\
         3. 300 字以内，直接输出提要正文。\n\
         ---\n{}",
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
}
