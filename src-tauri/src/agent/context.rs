use serde_json::{json, Value};

use crate::api::OpenRouterClient;

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

pub fn needs_compression(messages: &[Value], context_limit: usize) -> bool {
    history_tokens(messages) as f64 >= context_limit as f64 * COMPRESSION_THRESHOLD
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

pub async fn compress(
    client: &OpenRouterClient,
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

    let summary = client
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
        assert!(!needs_compression(&messages, 2000));
        let messages = vec![msg("system", &"x".repeat(6800))];
        assert!(needs_compression(&messages, 2000));
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
