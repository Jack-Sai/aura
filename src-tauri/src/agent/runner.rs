use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::api::Router;
use crate::tools;
use super::context::{compress, estimate_tokens, local_trim, needs_compression};
use super::parser::{Event, Finish, Parser};
use super::prompt::build_system_prompt;

pub const EV_CHUNK: &str = "agent:chunk";
pub const EV_ACTION: &str = "agent:action";
pub const EV_NOTICE: &str = "agent:notice";
pub const EV_ERROR: &str = "agent:error";
pub const EV_DONE: &str = "agent:done";

const MAX_TOOL_ROUNDS: usize = 50;
const MAX_PARSE_RETRIES: u32 = 3;

pub struct TurnContext<'a> {
    pub app: &'a AppHandle,
    pub router: &'a Router,
    pub workspace: &'a Path,
    pub global_rules: &'a str,
    pub history: &'a mut Vec<Value>,
    pub model_idx: &'a mut usize,
    pub session_id: &'a str,
    pub cancelled: &'a AtomicBool,
}

enum Next {
    Tool(String, Value),
    Reject(String),
}

fn emit_text(app: &AppHandle, session_id: &str, event: &str, text: &str) {
    let _ = app.emit(event, json!({ "session_id": session_id, "text": text }));
}

fn emit_done(app: &AppHandle, session_id: &str) {
    let _ = app.emit(EV_DONE, json!({ "session_id": session_id }));
}

fn emit_cancelled(app: &AppHandle, session_id: &str) {
    emit_text(app, session_id, EV_NOTICE, "已停止生成");
    emit_done(app, session_id);
}

fn push_assistant(history: &mut Vec<Value>, content: &str) {
    if !content.is_empty() {
        history.push(json!({ "role": "assistant", "content": content }));
    }
}

fn truncate_tail(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let tail: String = chars[chars.len() - max..].iter().collect();
    format!("…{}", tail)
}

fn normalize_tool_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .replace('-', "_")
        .replace(' ', "_")
}

fn parse_tool_call(raw: &str) -> Result<(String, Value), String> {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.trim_start_matches(['\n', '\r']);
        s = match rest.rfind("```") {
            Some(end) => rest[..end].trim_end(),
            None => rest,
        };
    }
    let value: Value =
        serde_json::from_str(s).map_err(|e| format!("JSON parse failed - {}", e))?;
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "JSON parse failed - missing field `name`".to_string())?
        .to_string();
    let args = value.get("args").cloned().unwrap_or_else(|| json!({}));
    Ok((name, args))
}

fn is_known_tool(name: &str) -> bool {
    matches!(
        normalize_tool_name(name).as_str(),
        "list_files" | "read_file" | "search_workspace"
    )
}

fn write_diag_log(info: &str) {
    let path = std::env::temp_dir().join("aura-diag.log");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {}", now, info);
        let _ = writeln!(f, "----");
    }
}

pub async fn run_turn(ctx: &mut TurnContext<'_>, user_message: &str) -> Result<(), String> {
    ctx.history
        .push(json!({ "role": "user", "content": user_message }));
    let mut parse_failures: u32 = 0;
    let mut consecutive: Option<(String, usize)> = None;
    let mut rounds = 0usize;

    loop {
        if ctx.cancelled.load(Ordering::Relaxed) {
            emit_cancelled(ctx.app, ctx.session_id);
            return Ok(());
        }
        rounds += 1;
        if rounds > MAX_TOOL_ROUNDS {
            emit_text(
                ctx.app,
                ctx.session_id,
                EV_ERROR,
                "已达到最大工具轮次，请基于已有信息直接回答。",
            );
            return Err("max tool rounds exceeded".into());
        }

        let limit = ctx.router.context_limit(*ctx.model_idx);
        let ws_display = ctx.workspace.display().to_string().replace('\\', "/");
        let system = build_system_prompt(&ws_display, ctx.global_rules);
        let system_tokens = estimate_tokens(&system) + 4;
        if needs_compression(ctx.history, limit, system_tokens) {
            match compress(ctx.router, *ctx.model_idx, ctx.history, limit).await {
                Some(new) => {
                    *ctx.history = new;
                }
                None => {
                    // 摘要请求失败（网络/限流）时本地兜底截断，防止 Token 溢出
                    let trimmed = local_trim(ctx.history, limit, system_tokens);
                    if trimmed.len() < ctx.history.len() {
                        emit_text(
                            ctx.app,
                            ctx.session_id,
                            EV_NOTICE,
                            "上下文接近上限，摘要生成失败，已截断较早的消息",
                        );
                        *ctx.history = trimmed;
                    }
                }
            }
            // 摘要或截断后若仍超阈值（单条消息过大），再次本地截断保证不溢出
            if needs_compression(ctx.history, limit, system_tokens) {
                let trimmed = local_trim(ctx.history, limit, system_tokens);
                if trimmed.len() < ctx.history.len() {
                    *ctx.history = trimmed;
                }
            }
        }
        let mut messages = Vec::with_capacity(ctx.history.len() + 1);
        messages.push(json!({ "role": "system", "content": system }));
        messages.extend(ctx.history.iter().cloned());

        let fb = match ctx.router.stream_with_fallback(*ctx.model_idx, &messages).await {
            Ok(fb) => fb,
            Err(e) => {
                emit_text(ctx.app, ctx.session_id, EV_ERROR, &format!("请求失败：{}", e));
                return Err(format!("api error: {}", e));
            }
        };
        if let Some(notice) = fb.notice {
            emit_text(ctx.app, ctx.session_id, EV_NOTICE, &notice);
        }
        *ctx.model_idx = fb.model_idx;

        let mut parser = Parser::new();
        let mut turn_output = String::new();
        let mut tool_json: Option<String> = None;
        let mut answered = false;
        let mut stream_err: Option<String> = None;

        let mut stream = fb.value;
        let mut was_cancelled = false;
        'stream: loop {
            let cancel = ctx.cancelled;
            let item = tokio::select! {
                i = stream.next() => i,
                _ = async {
                    while !cancel.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } => {
                    was_cancelled = true;
                    break 'stream;
                }
            };
            match item {
                Some(Ok(text)) => {
                    let mut events = Vec::new();
                    parser.feed(&text, &mut events);
                    for ev in events {
                        match ev {
                            Event::Chunk(t) => {
                                turn_output.push_str(&t);
                                emit_text(ctx.app, ctx.session_id, EV_CHUNK, &t);
                            }
                            Event::Tool(json_str) => {
                                turn_output
                                    .push_str(&format!("<tool>{}</tool>", json_str));
                                tool_json = Some(json_str);
                                break 'stream;
                            }
                            Event::AnswerDone => {
                                turn_output.push_str("</answer>");
                                answered = true;
                                break 'stream;
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    stream_err = Some(e.to_string());
                    break;
                }
                None => break,
            }
        }

        if was_cancelled {
            push_assistant(ctx.history, &turn_output);
            emit_cancelled(ctx.app, ctx.session_id);
            return Ok(());
        }

        if let Some(msg) = stream_err {
            push_assistant(ctx.history, &turn_output);
            emit_text(ctx.app, ctx.session_id, EV_ERROR, &format!("连接中断：{}", msg));
            return Err(msg);
        }

        if answered {
            push_assistant(ctx.history, &turn_output);
            emit_done(ctx.app, ctx.session_id);
            return Ok(());
        }

        if tool_json.is_none() {
            let mut tail = Vec::new();
            let finish = parser.finish(&mut tail);
            for ev in tail {
                match ev {
                    Event::Chunk(t) => {
                        turn_output.push_str(&t);
                        emit_text(ctx.app, ctx.session_id, EV_CHUNK, &t);
                    }
                    Event::Tool(json_str) => {
                        turn_output.push_str(&format!("<tool>{}</tool>", json_str));
                        tool_json = Some(json_str);
                    }
                    Event::AnswerDone => {
                        turn_output.push_str("</answer>");
                        answered = true;
                    }
                }
            }
            if answered {
                push_assistant(ctx.history, &turn_output);
                emit_done(ctx.app, ctx.session_id);
                return Ok(());
            }
            if tool_json.is_none() {
                // content 为空但模型输出了 reasoning（思考）时，以思考内容兜底
                if turn_output.is_empty() {
                    let reasoning = stream.take_reasoning();
                    if !reasoning.is_empty() {
                        turn_output.push_str(&reasoning);
                        emit_text(
                            ctx.app,
                            ctx.session_id,
                            EV_NOTICE,
                            "本轮仅有思考过程，已将其作为内容展示",
                        );
                    }
                }
                // 模型未输出 <answer> 标签，但流正常结束且已有正文：
                // 视为模型直接作答，宽容接受，不再强制协议标签。
                if !turn_output.is_empty() {
                    push_assistant(ctx.history, &turn_output);
                    emit_done(ctx.app, ctx.session_id);
                    return Ok(());
                }
                {
                    let (count, tail) = stream.diag();
                    write_diag_log(&format!(
                        "empty output; session={}; events={};\nraw tail:\n{}",
                        ctx.session_id, count, tail
                    ));
                }
                push_assistant(ctx.history, &turn_output);
                parse_failures += 1;
                if parse_failures > MAX_PARSE_RETRIES {
                    emit_text(ctx.app, ctx.session_id, EV_ERROR, "模型解析失败，请重试。");
                    return Err("model output parse failed".into());
                }
                let msg = match finish {
                    Finish::UnclosedTool => "JSON parse failed - tool tag is not closed",
                    _ => "输出未包含合法的 <tool> 或 <answer> 标签，请严格按照协议输出",
                };
                ctx.history.push(json!({
                    "role": "user",
                    "content": format!("<tool_result>Error: {}</tool_result>", msg)
                }));
                continue;
            }
        }

        let next = {
            let raw = tool_json.clone().unwrap_or_default();
            match parse_tool_call(&raw) {
                Ok((name, args)) if is_known_tool(&name) => {
                    Next::Tool(normalize_tool_name(&name), args)
                }
                Ok((name, _)) => Next::Reject(format!(
                    "unknown tool '{}'，可用工具为 list_files、read_file、search_workspace",
                    name
                )),
                Err(msg) => Next::Reject(msg),
            }
        };

        match next {
            Next::Reject(msg) => {
                push_assistant(ctx.history, &turn_output);
                emit_text(
                    ctx.app,
                    ctx.session_id,
                    EV_NOTICE,
                    &format!(
                        "工具调用无效：{}；原文：{}",
                        msg,
                        truncate_tail(&turn_output, 200)
                    ),
                );
                parse_failures += 1;
                if parse_failures > MAX_PARSE_RETRIES {
                    if !turn_output.is_empty() {
                        emit_text(
                            ctx.app,
                            ctx.session_id,
                            EV_NOTICE,
                            "多次工具调用无效，已将当前输出作为回答",
                        );
                        emit_done(ctx.app, ctx.session_id);
                        return Ok(());
                    }
                    emit_text(ctx.app, ctx.session_id, EV_ERROR, "模型解析失败，请重试。");
                    return Err("model output parse failed".into());
                }
                ctx.history.push(json!({
                    "role": "user",
                    "content": format!("<tool_result>Error: {}</tool_result>", msg)
                }));
                continue;
            }
            Next::Tool(name, args) => {
                push_assistant(ctx.history, &turn_output);
                parse_failures = 0;

                let over_limit = matches!(
                    &consecutive,
                    Some((prev, count)) if *prev == name && *count >= 5
                );
                consecutive = match consecutive {
                    Some((prev, count)) if prev == name => Some((name.clone(), count + 1)),
                    _ => Some((name.clone(), 1)),
                };

                let body = if over_limit {
                    "Error: 连续调用次数超限，请尝试其他工具或基于现有信息直接回复。"
                        .to_string()
                } else {
                    match tools::execute(ctx.workspace, &name, &args) {
                        Ok(out) => {
                            emit_text(ctx.app, ctx.session_id, EV_ACTION, &out.label);
                            out.result
                        }
                        Err(fail) => {
                            emit_text(ctx.app, ctx.session_id, EV_ACTION, &fail.label);
                            format!("Error: {}", fail.error)
                        }
                    }
                };
                ctx.history.push(json!({
                    "role": "user",
                    "content": format!("<tool_result>{}</tool_result>", body)
                }));
                continue;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_call_plain() {
        let (name, args) =
            parse_tool_call(r#"{"name": "read_file", "args": {"path": "a.md"}}"#).unwrap();
        assert_eq!(name, "read_file");
        assert_eq!(args["path"], "a.md");
    }

    #[test]
    fn parse_tool_call_strips_markdown_fence() {
        let raw = "```json\n{\"name\": \"list_files\", \"args\": {\"path\": \"/\"}}\n```";
        let (name, _) = parse_tool_call(raw).unwrap();
        assert_eq!(name, "list_files");
    }

    #[test]
    fn tool_name_normalization_accepts_variants() {
        assert!(is_known_tool("Read-File"));
        assert!(is_known_tool("read file"));
        assert!(is_known_tool("READ_FILE"));
        assert!(!is_known_tool("write_file"));
        assert_eq!(normalize_tool_name(" Read-File "), "read_file");
    }

    #[test]
    fn truncate_tail_keeps_suffix() {
        assert_eq!(truncate_tail("短文本", 200), "短文本");
        let long = "a".repeat(300);
        let out = truncate_tail(&long, 200);
        assert_eq!(out.chars().count(), 201);
        assert!(out.starts_with('…'));
    }
}