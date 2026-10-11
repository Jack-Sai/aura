use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::api::Router;
use crate::tools;
use super::context::{compress, estimate_tokens, local_trim, needs_compression};
use super::keepawake::KeepAwake;
use super::mode::Mode;
use super::parser::{Event, Finish, Parser};
use super::prompt::{
    build_compact_system_prompt, build_system_prompt, is_small_model, todo_update_brief,
};
use super::todo;
use super::{Todo, TodoStatus};

pub const EV_CHUNK: &str = "agent:chunk";
pub const EV_ACTION: &str = "agent:action";
pub const EV_NOTICE: &str = "agent:notice";
pub const EV_ERROR: &str = "agent:error";
pub const EV_DONE: &str = "agent:done";
/// 规划模式产出的任务清单。载荷为清洗后的 Todo 数组。
pub const EV_TODO_PLAN: &str = "agent:todo_plan";
/// 单个步骤的状态推进。载荷为 `{id, status}`。
pub const EV_TODO_UPDATE: &str = "agent:todo_update";

/// 单轮对话允许的最大工具轮次。
///
/// v0.3.0 之前是 50，但写入任务一次完整闭环（读文件 → 改 → 编译 → 读报错
/// → 再改 → 再编译…）轻易打满，表现为「模型被迫中止」。按每轮一次工具
/// 调用估算，100 轮足以覆盖绝大多数真实任务。
const MAX_TOOL_ROUNDS: usize = 100;
const MAX_PARSE_RETRIES: u32 = 3;
const MAX_PARSE_RETRIES_SMALL: u32 = 1;

/// 连续调用节流键：工具名 + 作用目标。
///
/// 仅按工具名计数会误伤合法的批量修改——例如连续 10 次改同一个文件，
/// 按名字计数会在第 6 次被无端拦截。加上目标后，「改 A 文件」与
/// 「改 B 文件」互不干扰，而对同一文件反复死循环仍会被拦。
fn throttle_key(name: &str, args: &Value) -> String {
    let target = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "*".to_string());
    format!("{}|{}", name, target)
}

pub struct TurnContext<'a> {
    pub app: &'a AppHandle,
    pub router: &'a Router,
    pub workspace: &'a Path,
    pub global_rules: &'a str,
    pub history: &'a mut Vec<Value>,
    pub model_idx: &'a mut usize,
    pub session_id: &'a str,
    pub cancelled: &'a AtomicBool,
    /// 本轮会话模式，决定可用工具与 prompt 形态。默认 Chat 以兼容既有调用方。
    pub mode: Mode,
    /// 当前任务清单的运行时副本。
    ///
    /// 用 `&mut` 是关键：runner 每轮工具调用后都会重建 system prompt，
    /// 清单在此原地更新后，下一轮自动带上最新进度，无需额外同步机制。
    pub todos: &'a mut Vec<Todo>,
    /// 落盘任务清单的回调。
    ///
    /// runner 不持有数据库连接，由 `commands.rs` 注入闭包，
    /// 这样存储细节不会渗进 agent 层，也便于测试时替换。
    pub persist_todos: Option<&'a (dyn Fn(Vec<Todo>) + Send + Sync)>,
}

enum Next {
    Tool(String, Value),
    Reject(String),
}

fn emit_text(app: &AppHandle, session_id: &str, event: &str, text: &str) {
    let _ = app.emit(event, json!({ "session_id": session_id, "text": text }));
}

/// 工具动作事件。`ok` 区分成功与失败；`detail` 携带可折叠的详细输出
/// （如命令执行的 stdout/stderr），过长时不应直接铺满对话流。
fn emit_action_detail(
    app: &AppHandle,
    session_id: &str,
    label: &str,
    ok: bool,
    detail: Option<String>,
) {
    let _ = app.emit(
        EV_ACTION,
        json!({ "session_id": session_id, "text": label, "ok": ok, "detail": detail }),
    );
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
    crate::tools::is_known(&normalize_tool_name(name))
}

/// 处理模型产出的 `<todo_plan>`：解析、落库、推送前端。
///
/// 解析失败不打断本轮对话——用户仍应看到模型的文字说明，
/// 只是拿不到结构化清单。失败原因通过 notice 回传，让用户知道
/// 「计划没生成出来」而不是以为模型没写计划。
fn handle_plan(ctx: &TurnContext<'_>, raw: &str) {
    let parsed = match todo::parse_plan(raw) {
        Ok(p) => p,
        Err(msg) => {
            emit_text(
                ctx.app,
                ctx.session_id,
                EV_NOTICE,
                &format!("未能解析任务清单：{msg}"),
            );
            return;
        }
    };
    if parsed.dropped > 0 {
        emit_text(
            ctx.app,
            ctx.session_id,
            EV_NOTICE,
            &format!("任务清单中有 {} 条缺少标题或 id 重复，已忽略。", parsed.dropped),
        );
    }
    if let Some(app) = ctx.persist_todos {
        app(parsed.todos.clone());
    }
    let _ = ctx.app.emit(
        EV_TODO_PLAN,
        json!({ "session_id": ctx.session_id, "todos": parsed.todos }),
    );
}

/// 处理模型产出的 `<todo_update>`：应用状态变更、落库、推送前端。
///
/// 与计划解析不同，这里**静默失败**：清单是辅助信息，模型偶尔漏更新
/// 或引用了已删除的步骤都不该打断对话。用户看到的清单不更新，
/// 总好过对话中断。
fn handle_todo_update(ctx: &mut TurnContext<'_>, raw: &str) {
    let value: Value = match serde_json::from_str(raw.trim()) {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some(id) = value.get("id").and_then(|v| v.as_str()).map(str::trim) else {
        return;
    };
    if id.is_empty() {
        return;
    }
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .map(TodoStatus::parse)
        .unwrap_or_default();

    // 直接改 ctx.todos：它是 &mut 引用，后续每轮重建 prompt 时
    // 都会自动带上最新进度，无需额外的同步机制
    if !todo::apply_update(ctx.todos, id, status) {
        return;
    }
    if let Some(app) = ctx.persist_todos {
        app(ctx.todos.to_vec());
    }
    let _ = ctx.app.emit(
        EV_TODO_UPDATE,
        json!({ "session_id": ctx.session_id, "id": id, "status": status.as_str() }),
    );
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
    // 执行期间阻止系统休眠：耗时命令（编译/测试）被挂起会导致任务假死
    let _keep_awake = KeepAwake::new();
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

        // 获取当前模型 ID 以判断是否为小模型
        let model_id = {
            let cfg = ctx.router.config();
            let idx = *ctx.model_idx;
            let idx = idx.min(cfg.models.len().saturating_sub(1));
            cfg.models
                .get(idx)
                .map(|m| m.id.clone())
                .unwrap_or_default()
        };
        let is_small = is_small_model(&model_id);
        let max_parse_retries = if is_small { MAX_PARSE_RETRIES_SMALL } else { MAX_PARSE_RETRIES };

        // 小模型使用精简 System Prompt，减少 Token 消耗。
        // 规划模式例外：小模型虽难执行多轮写入，但产出计划正是它的强项，
        // 因此仍走完整 prompt（工具清单会按模式过滤为只读）。
        let system = if is_small && ctx.mode != Mode::Plan {
            build_compact_system_prompt(&ws_display, ctx.global_rules)
        } else {
            build_system_prompt(&ws_display, ctx.global_rules, ctx.mode)
        };
        // 清单在每轮重建时注入：模型刚更新完的进度会在下一次工具调用时
        // 立即反映到 prompt，无需额外的同步或缓存
        let system = if ctx.todos.is_empty() {
            system
        } else {
            let mut s = system;
            // 小模型走的是精简 prompt，里面没有 todo_update 协议。
            // 但清单已是进行中的状态，缺了协议就会从头到尾不更新一次
            if is_small {
                s.push_str(todo_update_brief());
            }
            s.push_str(&todo::render_for_prompt(ctx.todos));
            s
        };
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

        // TTFT 反馈：本地模型推理较慢，提前告知用户
        if ctx.router.is_local_model(*ctx.model_idx) {
            emit_text(
                ctx.app,
                ctx.session_id,
                EV_NOTICE,
                "正在向本地模型推理，首字可能需要较长时间...",
            );
        }

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
                            Event::TodoPlan(json_str) => {
                                // 保留原文进 history：既让模型知道计划已输出，
                                // 也保证后续轮次能看到自己写过什么
                                turn_output
                                    .push_str(&format!("<todo_plan>{}</todo_plan>", json_str));
                                handle_plan(ctx, &json_str);
                                // 注意：这里**不能** break 'stream。
                                // 计划之后模型还要输出 <answer> 做说明，
                                // 打断流会导致那段正文整个丢失。
                            }
                            Event::TodoUpdate(json_str) => {
                                turn_output
                                    .push_str(&format!("<todo_update>{}</todo_update>", json_str));
                                handle_todo_update(ctx, &json_str);
                                // 同样不能 break 'stream：Build 模式的正常形态是
                                // 「更新一步 → 调工具 → 更新一步」，打断即丢失后续内容
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
                    Event::TodoPlan(json_str) => {
                        turn_output
                            .push_str(&format!("<todo_plan>{}</todo_plan>", json_str));
                        handle_plan(ctx, &json_str);
                    }
                    Event::TodoUpdate(json_str) => {
                        turn_output
                            .push_str(&format!("<todo_update>{}</todo_update>", json_str));
                        handle_todo_update(ctx, &json_str);
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
                if parse_failures > max_parse_retries {
                    emit_text(ctx.app, ctx.session_id, EV_ERROR, "模型解析失败，请重试。");
                    return Err("model output parse failed".into());
                }
                let msg = match finish {
                    Finish::UnclosedTool => "JSON parse failed - tool tag is not closed",
                    Finish::UnclosedPlan => {
                        "todo_plan tag is not closed - 请输出完整的 <todo_plan>...</todo_plan>"
                    }
                    Finish::UnclosedTodo => {
                        "todo_update tag is not closed - 请输出完整的 <todo_update>...</todo_update>"
                    }
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
                Ok((name, args)) => {
                    let norm = normalize_tool_name(&name);
                    if !is_known_tool(&norm) {
                        Next::Reject(format!(
                            "unknown tool '{}'，可用工具为 {}",
                            name,
                            crate::tools::registry::names().join("、")
                        ))
                    } else if !ctx.mode.allows(&norm) {
                        // 模式隔离的硬闸门：prompt 已声明 Plan 只读，但模型
                        // 仍可能幻觉调用写工具。此处拒绝并回传可操作的提示，
                        // 让模型改用只读方式继续，而不是静默丢弃。
                        Next::Reject(ctx.mode.reject_reason(&norm))
                    } else {
                        Next::Tool(norm, args)
                    }
                }
                Err(msg) => Next::Reject(msg),
            }
        };

        match next {
            Next::Reject(msg) => {
                push_assistant(ctx.history, &turn_output);
                // 工具压根没执行成功，不应计入连续调用节流
                consecutive = None;
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
                if parse_failures > max_parse_retries {
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

                let tool_label = tools::tool_label(&name, &args);
                // 节流按「工具 + 目标」计数，并使用注册表声明的上限
                let throttle = throttle_key(&name, &args);
                let limit = tools::registry::max_consecutive(&name);
                let over_limit = matches!(
                    &consecutive,
                    Some((prev, count)) if *prev == throttle && *count >= limit as usize
                );
                consecutive = match consecutive {
                    Some((prev, count)) if prev == throttle => {
                        Some((throttle, count + 1))
                    }
                    _ => Some((throttle, 1)),
                };

                let body = if over_limit {
                    emit_action_detail(
                        ctx.app,
                        ctx.session_id,
                        &format!("{}（连续调用超限）", tool_label),
                        false,
                        None,
                    );
                    "Error: 连续调用次数超限，请尝试其他工具或基于现有信息直接回复。"
                        .to_string()
                } else {
                    match tools::execute(
                        ctx.workspace.to_path_buf(),
                        name.clone(),
                        args.clone(),
                    )
                    .await
                    {
                        Ok(out) => {
                            // Exec 类工具的结果是命令输出，交给前端折叠展示
                            let detail = if name.starts_with("run_script") {
                                Some(out.result.clone())
                            } else {
                                None
                            };
                            emit_action_detail(
                                ctx.app,
                                ctx.session_id,
                                &out.label,
                                true,
                                detail,
                            );
                            out.result
                        }
                        Err(fail) => {
                            // 失败信息始终附带，便于用户看清原因
                            emit_action_detail(
                                ctx.app,
                                ctx.session_id,
                                &fail.label,
                                false,
                                Some(fail.error.clone()),
                            );
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
    fn throttle_key_distinguishes_targets() {
        use serde_json::json;
        // 同一工具不同目标 → 不同键，互不干扰
        let a = throttle_key("write_file", &json!({ "path": "a.txt" }));
        let b = throttle_key("write_file", &json!({ "path": "b.txt" }));
        assert_ne!(a, b);
        // 同一工具同一目标 → 同键，死循环仍会被拦
        assert_eq!(a, throttle_key("write_file", &json!({ "path": "a.txt" })));
        // 不同工具同一目标 → 不同键
        assert_ne!(
            throttle_key("write_file", &json!({ "path": "a.txt" })),
            throttle_key("read_file", &json!({ "path": "a.txt" }))
        );
        // 无 path 参数（如 search_workspace）归入通配目标
        let s1 = throttle_key("search_workspace", &json!({ "query": "x" }));
        let s2 = throttle_key("search_workspace", &json!({ "query": "y" }));
        assert_eq!(s1, s2);
    }

    #[test]
    fn tool_name_normalization_accepts_variants() {
        assert!(is_known_tool("Read-File"));
        assert!(is_known_tool("read file"));
        assert!(is_known_tool("READ_FILE"));
        assert_eq!(normalize_tool_name(" Read-File "), "read_file");
        // 未注册的工具应被拒绝；判定与注册表联动，不硬编码工具名
        assert!(!is_known_tool("definitely_not_a_tool"));
        // 每个已注册工具的原始名与归一化名都应通过
        for name in crate::tools::registry::names() {
            assert!(is_known_tool(name), "注册表工具未通过校验: {}", name);
        }
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