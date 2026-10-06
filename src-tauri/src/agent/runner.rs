use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::api::{MODELS, OpenRouterClient};
use crate::tools;
use super::context::{compress, needs_compression};
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
    pub client: &'a OpenRouterClient,
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

fn parse_tool_call(raw: &str) -> Result<(String, Value), String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("JSON parse failed - {}", e))?;
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "JSON parse failed - missing field `name`".to_string())?
        .to_string();
    let args = value.get("args").cloned().unwrap_or_else(|| json!({}));
    Ok((name, args))
}

fn is_known_tool(name: &str) -> bool {
    matches!(name, "list_files" | "read_file" | "search_workspace")
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

        let limit = MODELS[*ctx.model_idx].context_limit;
        if needs_compression(ctx.history, limit) {
            if let Some(new) = compress(ctx.client, *ctx.model_idx, ctx.history, limit).await {
                *ctx.history = new;
            }
        }

        let ws_display = ctx.workspace.display().to_string().replace('\\', "/");
        let system = build_system_prompt(&ws_display, ctx.global_rules);
        let mut messages = Vec::with_capacity(ctx.history.len() + 1);
        messages.push(json!({ "role": "system", "content": system }));
        messages.extend(ctx.history.iter().cloned());

        let fb = match ctx.client.stream_with_fallback(*ctx.model_idx, &messages).await {
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
                Ok((name, args)) if is_known_tool(&name) => Next::Tool(name, args),
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
                parse_failures += 1;
                if parse_failures > MAX_PARSE_RETRIES {
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
