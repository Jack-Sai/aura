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

/// 流空闲上限：连续多久收不到任何数据就判定上游失联。
///
/// 之前 `select!` 只有「拿到数据」与「用户手动中断」两条出路，
/// 而 SSE 在无数据时永久 `Pending`（见 `sse.rs`）。只要连接建好后模型
/// 停顿一下，`stream.next()` 就永不返回，界面零输出地挂住，
/// 只能靠用户手动中断——这是 v0.4.4 修复的核心问题。
///
/// 90 秒的取值依据：本地 7B 级模型单次推理偶尔可达 30~60 秒，
/// 再叠加首 token 延迟，90 秒仍属异常；真正的死连接通常在
/// TCP 层很快就会断开，不会占用这么久。
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// 单轮内最多接受多少个工具调用（v0.5.3）。
///
/// 模型一轮给出多个 `<tool>` 是现在推荐的写法（它们会**依次**执行，
/// 存在前后依赖也能放在同一轮），但仍需设上限兜底：一次甩出几十个调用
/// 会让节流计数失去意义，并把大量结果一次性塞进历史挤占上下文。
/// 超出上限的调用会被丢弃，模型看到结果数少于自己发出的调用数时会自行分批。
const MAX_TOOLS_PER_TURN: usize = 8;

/// 连续调用节流键：工具名 + 作用目标。
///
/// 仅按工具名计数会误伤合法的批量修改——例如连续 10 次改同一个文件，
/// 按名字计数会在第 6 次被无端拦截。加上目标后，「改 A 文件」与
/// 「改 B 文件」互不干扰，而对同一文件反复死循环仍会被拦。
fn throttle_key(name: &str, args: &Value) -> String {
    let target = tool_target(name, args);
    if target.is_empty() {
        format!("{}|*", name)
    } else {
        format!("{}|{}", name, target)
    }
}

/// 从调用参数里提取一个短且可读的作用目标，用于节流键与回填标签。
///
/// 只有 `path` 参与节流判定（改动不同文件本就该放行）；
/// 命令与检索词本身没有可寻址的资源，提取出来只用于展示，
/// 因此 `throttle_key` 拿到它们时仍应回落到 `*`。
fn tool_target(name: &str, args: &Value) -> String {
    args.get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(60).collect())
        .unwrap_or_else(|| {
            if name.starts_with("run_script") {
                args.get("command")
                    .and_then(|v| v.as_str())
                    .map(|c| {
                        c.lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .chars()
                            .take(40)
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                args.get("query")
                    .and_then(|v| v.as_str())
                    .map(|q| q.chars().take(40).collect())
                    .unwrap_or_default()
            }
        })
}

/// 生成一条带标识的 `<tool_result>`。
///
/// **必须带 `name`**：一轮里可能同时回填多个结果，模型要能分辨
/// 哪个结果对应哪个调用——否则某个失败时它会重试整批。
/// 即便一轮只有一个工具，这个标识同样有用：失败信息能明确指向具体调用。
fn tool_result_tag(name: &str, args: &Value, body: &str) -> String {
    let target = tool_target(name, args);
    if target.is_empty() {
        format!("<tool_result name=\"{}\">{}</tool_result>", name, body)
    } else {
        format!(
            "<tool_result name=\"{}\" target=\"{}\">{}</tool_result>",
            name,
            target.replace('"', "'"),
            body
        )
    }
}

/// 工具拒绝计数：按工具名独立统计。
///
/// 此前只用一个全局 `parse_failures`，且每次工具成功放行就清零。
/// 于是模型陷入「write_file 被拒 → read_file 成功 → write_file 被拒」
/// 的交替时，计数永远到不了阈值，唯一刹车变成 `MAX_TOOL_ROUNDS=100`
/// ——本地模型每轮数秒，单轮可能跑几分钟，用户只看到界面一直不响应。
///
/// 改为按工具名分别计数：同一个被拒工具累计到上限即终止，
/// 而「换个工具试试」不会被误伤。
#[derive(Default)]
struct RejectCounter {
    counts: std::collections::HashMap<String, u32>,
}

impl RejectCounter {
    /// 记录一次拒绝，返回是否已超过阈值。
    fn record(&mut self, tool: &str, limit: u32) -> bool {
        let c = self.counts.entry(tool.to_string()).or_insert(0);
        *c += 1;
        *c > limit
    }

    fn total(&self) -> u32 {
        self.counts.values().sum()
    }

    /// 清零某个工具的计数（该工具被成功执行后调用）。
    fn reset(&mut self, tool: &str) {
        self.counts.remove(tool);
    }
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

/// 把本轮已产生的内容补发给前端。
///
/// 中断路径下 `Event::Tool` 的 `break 'stream` 会让后续正文全部滞留
/// 在 `turn_output` 里——它进了 history（模型看得见）却从未 emit
/// （用户看不见）。结果是提示写着「已将当前输出作为回答」，而用户
/// 实际什么都没收到。这里剥掉协议标签后补发一次，让承诺兑现。
fn emit_pending_text(app: &AppHandle, session_id: &str, turn_output: &str) {
    let text = strip_protocol_tags(turn_output);
    if !text.trim().is_empty() {
        emit_text(app, session_id, EV_CHUNK, &text);
    }
}

/// 从一轮原始输出中提取可读正文，剔除协议标签与工具调用体。
///
/// `<tool>...</tool>` 是机器协议，逐字展示给用户毫无意义；
/// `<todo_plan>` / `<todo_update>` 已由侧边栏清单呈现，重复展示反而杂乱。
fn strip_protocol_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            // 未闭合的标签直接丢弃其后内容，避免把半截 JSON 当正文展示
            break;
        };
        let tag = &tail[..=end];
        let is_open = tag.starts_with("<tool")
            || tag.starts_with("<todo_plan")
            || tag.starts_with("<todo_update");
        // 闭合标签是机器协议的结尾，展示出来同样没有意义
        let is_close = tag.starts_with("</tool")
            || tag.starts_with("</todo_plan")
            || tag.starts_with("</todo_update")
            || tag.starts_with("</answer");
        if is_open {
            // 连同闭合标签与其中内容一起跳过
            let after = &tail[end + 1..];
            let closer = if tag.starts_with("<tool") {
                "</tool>"
            } else if tag.starts_with("<todo_plan") {
                "</todo_plan>"
            } else {
                "</todo_update>"
            };
            match after.find(closer) {
                Some(c) => {
                    out.push('\n');
                    rest = &after[c + closer.len()..];
                }
                None => {
                    rest = "";
                    break;
                }
            }
        } else if is_close {
            out.push('\n');
            rest = &tail[end + 1..];
        } else {
            out.push_str(tag);
            rest = &tail[end + 1..];
        }
    }
    if !rest.is_empty() {
        out.push_str(rest);
    }
    out
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
    match parse_plan_payload(raw) {
        Err(msg) => emit_text(
            ctx.app,
            ctx.session_id,
            EV_NOTICE,
            &format!("未能解析任务清单：{msg}"),
        ),
        Ok(PlanOutcome { todos, dropped }) => {
            if dropped > 0 {
                emit_text(
                    ctx.app,
                    ctx.session_id,
                    EV_NOTICE,
                    &format!("任务清单中有 {dropped} 条缺少标题或 id 重复，已忽略。"),
                );
            }
            if let Some(app) = ctx.persist_todos {
                app(todos.clone());
            }
            let _ = ctx.app.emit(
                EV_TODO_PLAN,
                json!({ "session_id": ctx.session_id, "todos": todos }),
            );
        }
    }
}

/// `<todo_plan>` 解析结果。
struct PlanOutcome {
    todos: Vec<Todo>,
    dropped: usize,
}

/// 解析计划载荷，不涉及任何副作用。
///
/// 从 [`handle_plan`] 里剥出来是为了能独立测试：整条链路此前只有
/// parser 层测过「切标签对不对」，而「切出来的载荷能不能变成清单、
/// 落库时带的是不是清洗后的版本」无人验证。
fn parse_plan_payload(raw: &str) -> Result<PlanOutcome, String> {
    let parsed = todo::parse_plan(raw)?;
    Ok(PlanOutcome {
        todos: parsed.todos,
        dropped: parsed.dropped,
    })
}

/// 解析并应用单条状态更新，返回应广播给前端的变更。
///
/// 返回 `None` 表示该更新应被静默忽略（格式非法、id 为空、步骤不存在）。
fn apply_update_payload(todos: &mut [Todo], raw: &str) -> Option<(String, TodoStatus)> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let id = value.get("id").and_then(|v| v.as_str()).map(str::trim)?;
    if id.is_empty() {
        return None;
    }
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .map(TodoStatus::parse)
        .unwrap_or_default();
    // 未命中不回传：模型可能引用已被用户删掉的步骤，静默忽略即可
    todo::apply_update(todos, id, status).then(|| (id.to_string(), status))
}

/// 处理模型产出的 `<todo_update>`：应用状态变更、落库、推送前端。
///
/// 与计划解析不同，这里**静默失败**：清单是辅助信息，模型偶尔漏更新
/// 或引用了已删除的步骤都不该打断对话。用户看到的清单不更新，
/// 总好过对话中断。
fn handle_todo_update(ctx: &mut TurnContext<'_>, raw: &str) {
    let Some((id, status)) = apply_update_payload(ctx.todos, raw) else {
        return;
    };
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

/// 是否使用精简 System Prompt。
///
/// 抽成纯函数是为了让这条规则可测——它决定小模型在执行模式下能否拿到
/// 写入工具，错了不会报错、只会「选了执行模式却什么都没发生」。
fn use_compact_prompt(is_small: bool, mode: Mode) -> bool {
    is_small && !mode.is_unrestricted()
}

/// 工具调用是否放行：已知且当前模式允许。
///
/// 与提示词是两套独立的闸门。提示词告诉模型「现在该做什么」，
/// 这里决定「它实际能做什么」——上下文被压缩、小模型幻觉时，
/// 提示词可能失效，但这道闸门不会。
fn tool_permitted(mode: Mode, name: &str) -> Result<(), String> {
    if !is_known_tool(name) {
        return Err(format!(
            "unknown tool '{name}'，可用工具为 {}",
            crate::tools::registry::names().join("、")
        ));
    }
    if !mode.allows(name) {
        return Err(mode.reject_reason(name));
    }
    Ok(())
}

pub async fn run_turn(ctx: &mut TurnContext<'_>, user_message: &str) -> Result<(), String> {
    // 执行期间阻止系统休眠：耗时命令（编译/测试）被挂起会导致任务假死
    let _keep_awake = KeepAwake::new();
    ctx.history
        .push(json!({ "role": "user", "content": user_message }));
    let mut parse_failures: u32 = 0;
        let mut rejects = RejectCounter::default();
    // v0.5.3：原先只记「上一个工具」的键+次数，一轮里给出多个工具时会互相
    // 覆盖——第二个把第一个的记录冲掉，第三个又覆盖第二个，节流形同虚设。
    // 改为按 throttle_key 分别累计。
    let mut consecutive: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
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
        // 但 Build 模式一律走完整 prompt：精简版只注入只读工具，
        // 那会让执行模式拿不到任何写入能力，模式语义直接落空。
        // 清单协议、写入纪律、退出码判读全在完整版里，也是最不能省的。
        let system = if use_compact_prompt(is_small, ctx.mode) {
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
        let mut tool_jsons: Vec<String> = Vec::new();
        let mut answered = false;
        let mut stream_err: Option<String> = None;

        let mut stream = fb.value;
        let mut was_cancelled = false;
        let mut idle_timed_out = false;
        'stream: loop {
            let cancel = ctx.cancelled;
            let item = tokio::select! {
                i = stream.next() => i,
                // 空闲超时：每收到一块数据就重置，因此只在真正失联时触发
                _ = tokio::time::sleep(STREAM_IDLE_TIMEOUT) => {
                    idle_timed_out = true;
                    break 'stream;
                }
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
                                // v0.5.3：**不打断流**，继续收集本轮的其余工具。
                                // 原先遇到第一个就 break，改 10 个文件就要 10 次
                                // 完整 LLM 往返；合批后一次往返即可，执行阶段
                                // 按写出的顺序依次跑，依赖关系天然满足。
                                // 收集到上限后仍继续消费流，但不再多收，
                                // 避免模型甩出几十个调用挤占上下文。
                                if tool_jsons.len() < MAX_TOOLS_PER_TURN {
                                    tool_jsons.push(json_str);
                                }
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

        // 上游长时间无响应。已收到的片段补发给用户——否则界面停在
        // 「什么都没发生」，而模型其实已经写了一半
        if idle_timed_out {
            push_assistant(ctx.history, &turn_output);
            emit_pending_text(ctx.app, ctx.session_id, &turn_output);
            emit_text(
                ctx.app,
                ctx.session_id,
                EV_ERROR,
                &format!("模型 {} 秒无响应，已中断本轮生成。", STREAM_IDLE_TIMEOUT.as_secs()),
            );
            emit_done(ctx.app, ctx.session_id);
            return Ok(());
        }

        if answered {
            push_assistant(ctx.history, &turn_output);
            emit_done(ctx.app, ctx.session_id);
            return Ok(());
        }

        if tool_jsons.is_empty() {
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
                        if tool_jsons.len() < MAX_TOOLS_PER_TURN {
                            tool_jsons.push(json_str);
                        }
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
if tool_jsons.is_empty() {
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

        // assistant 消息先入历史，与原先「处理前 push」的顺序保持一致；
        // 本轮全部工具跑完后，再把结果**聚合成一条** user 消息回填，
        // 保证顺序与模型写出的调用顺序严格对应。
        push_assistant(ctx.history, &turn_output);

        let mut results: Vec<String> = Vec::new();
        let mut first_reject_reported = false;

        for raw_json in &tool_jsons {
            let next = match parse_tool_call(raw_json) {
                Ok((name, args)) => {
                    let norm = normalize_tool_name(&name);
                    match tool_permitted(ctx.mode, &norm) {
                        Ok(()) => Next::Tool(norm, args),
                        Err(msg) => Next::Reject(msg),
                    }
                }
                Err(msg) => Next::Reject(msg),
            };

            match next {
                Next::Reject(msg) => {
                    // 工具压根没执行成功，不应计入连续调用节流
                    consecutive.clear();
                    // 一轮可能有多个调用被拒，但界面只需一次说明，
                    // 否则会刷出一屏重复的 error
                    if !first_reject_reported {
                        first_reject_reported = true;
                        // 提权为 error 事件：此前是灰色 notice，视觉权重极低，
                        // 用户容易把「工具被拒」误读成「界面卡住没反应」
                        emit_text(
                            ctx.app,
                            ctx.session_id,
                            EV_ERROR,
                            &format!(
                                "工具调用无效：{}；原文：{}",
                                msg,
                                truncate_tail(&turn_output, 200)
                            ),
                        );
                    }
                    // 按工具名独立计数：交替调用不同工具时无法收敛，
                    // 靠全局计数 + 成功即清零会一直跑到 MAX_TOOL_ROUNDS
                    let rejected = parse_tool_call(raw_json)
                        .map(|(name, _)| normalize_tool_name(&name))
                        .unwrap_or_else(|_| "unknown".to_string());
                    let exceeded = rejects.record(&rejected, max_parse_retries);
                    // 另一个兜底：全部拒绝次数也超限时终止，防止模型
                    // 轮换大量工具名无限重试
                    let total_exceeded = rejects.total() > max_parse_retries * 4;
                    if exceeded || total_exceeded {
                        emit_pending_text(ctx.app, ctx.session_id, &turn_output);
                        emit_text(
                            ctx.app,
                            ctx.session_id,
                            EV_ERROR,
                            &format!(
                                "`{rejected}` 连续被拒多次，已停止重试。可以换个说法，\
                                 或切换到执行模式后再试。"
                            ),
                        );
                        emit_done(ctx.app, ctx.session_id);
                        // 已产生的结果仍然回填，让模型知道自己错在哪
                        ctx.history.push(json!({ "role": "user", "content": results.join("\n") }));
                        return Ok(());
                    }
                    results.push(tool_result_tag(&rejected, &json!({}), &format!("Error: {}", msg)));
                }
                Next::Tool(name, args) => {
                    // 该工具已成功放行，清掉它的拒绝计数，
                    // 让「换个工具再试」不会继承旧账
                    rejects.reset(&name);

                    let tool_label = tools::tool_label(&name, &args);
                    // 节流按「工具 + 目标」计数，并使用注册表声明的上限。
                    // 一轮多个工具时各自独立累计，不会互相覆盖。
                    let throttle = throttle_key(&name, &args);
                    let limit = tools::registry::max_consecutive(&name);
                    let seen = consecutive.get(&throttle).copied().unwrap_or(0);
                    let over_limit = seen >= limit as usize;
                    consecutive.insert(throttle, seen + 1);

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
                    results.push(tool_result_tag(&name, &args, &body));
                }
            }
        }

        ctx.history.push(json!({
            "role": "user",
            "content": if results.is_empty() {
                "<tool_result>Error: 本轮没有任何可执行的工具调用</tool_result>".to_string()
            } else {
                results.join("\n")
            }
        }));
        continue;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 小模型在只读两档走精简 prompt，在执行档必须走完整 prompt。
    ///
    /// 这一条错了不会报错，只表现为「选了执行模式却什么都没发生」，
    /// 因此单独钉住。
    #[test]
    fn compact_prompt_only_applies_to_readonly_modes() {
        assert!(use_compact_prompt(true, Mode::Chat));
        assert!(use_compact_prompt(true, Mode::Plan));
        assert!(
            !use_compact_prompt(true, Mode::Build),
            "执行模式必须走完整 prompt，否则小模型拿不到任何写入工具"
        );
    }

    /// 大模型永远走完整 prompt，精简版只是为小模型省 token。
    #[test]
    fn large_models_always_use_full_prompt() {
        for mode in [Mode::Chat, Mode::Plan, Mode::Build] {
            assert!(
                !use_compact_prompt(false, mode),
                "{} 在大模型下不应走精简 prompt",
                mode.as_str()
            );
        }
    }

    #[test]
    fn parse_tool_call_plain() {
        let (name, args) =
            parse_tool_call(r#"{"name": "read_file", "args": {"path": "a.md"}}"#).unwrap();
        assert_eq!(name, "read_file");
        assert_eq!(args["path"], "a.md");
    }

    // ---- 拒绝计数：按工具名独立统计 ----

    #[test]
    fn reject_counter_isolates_tools() {
        let mut c = RejectCounter::default();
        // write_file 被拒两次、read_file 一次
        assert!(!c.record("write_file", 2));
        assert!(!c.record("write_file", 2));
        assert!(!c.record("read_file", 2));
        // write_file 第三次超限，但不应连带把 read_file 也算进来
        assert!(c.record("write_file", 2));
        assert_eq!(c.counts.get("read_file"), Some(&1));
    }

    /// 这是本次修复的核心：模型交替调用被拒工具与可成功工具时，
    /// 旧的「全局计数 + 成功即清零」永远不会收敛，能一直跑到
    /// MAX_TOOL_ROUNDS=100。现在按工具名独立计数可以收敛。
    #[test]
    fn reject_counter_terminates_despite_alternating_successes() {
        let mut c = RejectCounter::default();
        let limit = 3;
        let mut rounds = 0;
        loop {
            rounds += 1;
            // read_file 成功 → 只清它自己的计数
            c.reset("read_file");
            if c.record("write_file", limit) {
                break;
            }
            assert!(rounds < 20, "交替调用下计数必须收敛，否则会跑满 MAX_TOOL_ROUNDS");
        }
        assert!(rounds <= limit + 1);
    }

    #[test]
    fn reject_counter_total_bounds_tool_rotation() {
        // 模型轮换大量工具名重试时，总次数兜底必须生效
        let mut c = RejectCounter::default();
        for i in 0..20 {
            c.record(&format!("tool_{i}"), 100);
        }
        assert!(c.total() > 12, "总计数应能识别轮换工具名的重试");
    }

    // ---- v0.5.3：合批执行 ----

    /// 一轮多个工具时，回填必须逐条带上标识。
    ///
    /// 不带 name 时模型无法分辨哪个结果对应哪个调用——某个失败时它会重试整批。
    /// 单工具场景同样需要：失败信息能明确指向具体调用。
    #[test]
    fn tool_result_tag_carries_identity() {
        let args = json!({ "path": "src/main.rs" });
        let tag = tool_result_tag("read_file", &args, "内容");
        assert!(tag.contains("name=\"read_file\""), "缺少工具名: {}", tag);
        assert!(tag.contains("target=\"src/main.rs\""), "缺少作用目标: {}", tag);
        assert!(tag.contains(">内容"), "缺少结果体: {}", tag);

        // 无可寻址参数时只带 name
        let tag = tool_result_tag("search_workspace", &json!({}), "片段");
        assert!(tag.contains("name=\"search_workspace\""));
        assert!(!tag.contains("target="), "无目标时不应输出空 target: {}", tag);
    }

    /// 命令与检索词也要能提取目标，否则回填里只有工具名，模型难以分辨。
    #[test]
    fn tool_target_reads_command_and_query() {
        let cmd = json!({ "command": "cargo build\n--release" });
        assert_eq!(tool_target("run_script", &cmd), "cargo build");

        let q = json!({ "query": "连接池" });
        assert_eq!(tool_target("search_workspace", &q), "连接池");
    }

    /// 节流键：同工具不同目标互不干扰，同目标持续累积。
    ///
    /// 一轮给出多个工具时这条尤其关键——原先的「只记上一个」实现会互相覆盖。
    #[test]
    fn throttle_key_isolates_targets() {
        let a = throttle_key("write_file", &json!({ "path": "a.rs" }));
        let b = throttle_key("write_file", &json!({ "path": "b.rs" }));
        let a_again = throttle_key("write_file", &json!({ "path": "a.rs" }));
        assert_ne!(a, b, "不同目标不应共用节流计数");
        assert_eq!(a, a_again, "同一目标必须共用节流计数");

        // 无 path 的工具改用命令/检索词区分目标（同上）
        assert_ne!(
            throttle_key("run_script", &json!({ "command": "ls" })),
            throttle_key("run_script", &json!({ "command": "pwd" }))
        );
        // 完全取不到参数时回落 *
        assert_eq!(throttle_key("list_files", &json!({})), "list_files|*");
    }

    // ---- 中断收尾：补发已生成正文 ----

    #[test]
    fn strip_protocol_tags_removes_tool_bodies() {
        let raw = "开始处理。\n<tool>{\"name\":\"read_file\",\"args\":{\"path\":\"a.md\"}}</tool>\n读取完成。";
        let text = strip_protocol_tags(raw);
        assert!(text.contains("开始处理。"));
        assert!(text.contains("读取完成。"));
        assert!(
            !text.contains("read_file"),
            "工具调用体是机器协议，不应展示给用户"
        );
    }

    #[test]
    fn strip_protocol_tags_removes_todo_blocks() {
        // 清单已在侧边栏呈现，正文里重复展示只会杂乱
        let raw = "以下是计划：\n<todo_plan>{\"todos\":[{\"id\":\"s1\"}]}</todo_plan>\n请确认。";
        let text = strip_protocol_tags(raw);
        assert!(text.contains("以下是计划："));
        assert!(text.contains("请确认。"));
        assert!(!text.contains("todos"));
    }

    #[test]
    fn strip_protocol_tags_drops_unclosed_tag() {
        // 断流时标签可能未闭合，绝不能把半截 JSON 当正文展示
        let raw = "正常文字\n<tool>{\"name\":\"read_fi";
        let text = strip_protocol_tags(raw);
        assert!(text.contains("正常文字"));
        assert!(!text.contains("read_fi"));
    }

    #[test]
    fn strip_protocol_tags_keeps_plain_text_and_closing_tags() {
        assert_eq!(strip_protocol_tags("纯文本"), "纯文本");
        // 闭合标签是机器协议的结尾，同样不该展示
        assert!(!strip_protocol_tags("正文</answer>后续").contains("answer"));
    }

    // ---- 端到端：模型原始输出 → 解析器 → 载荷 → 清单状态 ----
    //
    // 此前整条链路只有 parser 层测过「标签切得对不对」，
    // 而「切出来的载荷能否变成清单」「落库的是不是清洗后的版本」
    // 无人验证。这里从原始模型输出开始跑完整条路径。

    /// 模拟模型输出：先调研、再给计划、最后用 <answer> 收尾。
    fn feed_model_output(parts: &[&str]) -> Vec<Event> {
        let mut parser = Parser::new();
        let mut events = Vec::new();
        for p in parts {
            parser.feed(p, &mut events);
        }
        parser.finish(&mut events);
        events
    }

    #[test]
    fn end_to_end_plan_reaches_clean_todo_list() {
        let events = feed_model_output(&[
            "我先看看项目结构。",
            "<tool>{\"name\":\"list_files\"}</tool>",
            "<todo_plan>{\"todos\":[{\"id\":\"s1\",\"title\":\"新增 config 模块\"},{\"id\":\"s2\",\"title\":\"注册路由\"}]}</todo_plan>",
            "<answer>计划如上</answer>",
        ]);

        // 从事件流里取出计划载荷，走与 handle_plan 相同的解析路径
        let raw = events
            .iter()
            .find_map(|e| match e {
                Event::TodoPlan(t) => Some(t.clone()),
                _ => None,
            })
            .expect("应解析出计划事件");

        let outcome = parse_plan_payload(&raw).expect("载荷应可解析");
        assert_eq!(outcome.todos.len(), 2);
        assert_eq!(outcome.dropped, 0);
        assert_eq!(outcome.todos[0].id, "s1");
        assert_eq!(outcome.todos[1].title, "注册路由");
        assert!(events.iter().any(|e| matches!(e, Event::AnswerDone)));
    }

    #[test]
    fn end_to_end_updates_drive_todo_progress() {
        let events = feed_model_output(&[
            "<todo_plan>{\"todos\":[{\"id\":\"s1\",\"title\":\"甲\"},{\"id\":\"s2\",\"title\":\"乙\"}]}</todo_plan>",
            "<todo_update>{\"id\":\"s1\",\"status\":\"in_progress\"}</todo_update>",
            "开始处理甲。",
            "<todo_update>{\"id\":\"s1\",\"status\":\"done\"}</todo_update>",
            "<todo_update>{\"id\":\"s2\",\"status\":\"done\"}</todo_update>",
            "<answer>全部完成</answer>",
        ]);

        let mut todos = vec![
            Todo::new("s1", "甲"),
            Todo::new("s2", "乙"),
        ];
        let mut updates = 0usize;
        for ev in &events {
            if let Event::TodoUpdate(raw) = ev {
                if let Some((id, status)) = apply_update_payload(&mut todos, raw) {
                    updates += 1;
                    assert!(todos.iter().any(|t| t.id == id));
                    assert!(todos.iter().find(|t| t.id == id).unwrap().status == status);
                }
            }
        }

        assert_eq!(updates, 3, "三条更新都应命中");
        assert_eq!(todo::done_count(&todos), 2);
        assert!(todos.iter().all(|t| t.is_done()));
    }

    /// 脏数据不得污染清单：坏 JSON、缺字段、指向不存在步骤。
    #[test]
    fn end_to_end_malformed_updates_are_ignored_silently() {
        let mut todos = vec![Todo::new("s1", "甲")];
        for raw in [
            "not json",
            "{}",
            r#"{"id":"","status":"done"}"#,
            r#"{"id":"nope","status":"done"}"#,
            "",
        ] {
            assert_eq!(
                apply_update_payload(&mut todos, raw),
                None,
                "脏载荷 {raw:?} 应被静默忽略"
            );
        }
        assert_eq!(todos.len(), 1);
        assert!(!todos[0].is_done(), "不得把状态写到别的步骤上");
    }

    // ---- 工具闸门：只读模式必须拦住一切写入 ----

    #[test]
    fn readonly_modes_block_mutating_tools_end_to_end() {
        // 模拟模型在对话模式下幻觉调用写工具
        let raw = r#"{"name":"write_file","args":{"path":"a.txt","content":"x"}}"#;
        let (name, _) = parse_tool_call(raw).unwrap();
        let norm = normalize_tool_name(&name);
        for mode in [Mode::Chat, Mode::Plan] {
            let err = tool_permitted(mode, &norm).unwrap_err();
            assert!(
                err.contains("write_file") && err.contains("执行模式"),
                "{} 的拒绝回执应指明工具与出路：{err}",
                mode.as_str()
            );
        }
    }

    #[test]
    fn build_mode_permits_mutating_tools_end_to_end() {
        for name in ["write_file", "string_replace", "run_script"] {
            assert!(
                tool_permitted(Mode::Build, name).is_ok(),
                "执行模式应放行 {name}"
            );
        }
    }

    #[test]
    fn unknown_tool_is_rejected_in_every_mode() {
        let err = tool_permitted(Mode::Build, "no_such_tool").unwrap_err();
        assert!(err.contains("unknown tool"));
        let err = tool_permitted(Mode::Chat, "no_such_tool").unwrap_err();
        assert!(err.contains("unknown tool"));
    }

    #[test]
    fn tool_names_are_normalized_before_the_gate() {
        // 模型可能写成 Write-File / WRITE_FILE / "write file"
        for raw in ["Write-File", "WRITE_FILE", "write file"] {
            let (name, _) =
                parse_tool_call(&format!(r#"{{"name":"{}"}}"#, raw.replace(' ', " "))).unwrap();
            let norm = normalize_tool_name(&name);
            assert_eq!(norm, "write_file", "{raw} 归一化后应为 write_file");
            assert!(
                tool_permitted(Mode::Chat, &norm).is_err(),
                "变体名不得绕过只读闸门"
            );
        }
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
        // v0.5.3：无 path 的工具改用命令/检索词作为目标。
        // 原先一律归入通配 `*`，于是 Build 模式连续执行 6 条**不同**命令
        // （build、test、lint…）就会因累计到 EXEC_LIMIT 被误判成「连续调用超限」，
        // 而它们之间毫无重复可言。
        assert_ne!(
            throttle_key("search_workspace", &json!({ "query": "x" })),
            throttle_key("search_workspace", &json!({ "query": "y" })),
            "不同检索词不该共用节流计数"
        );
        assert_ne!(
            throttle_key("run_script", &json!({ "command": "cargo build" })),
            throttle_key("run_script", &json!({ "command": "cargo test" })),
            "不同命令不该共用节流计数"
        );
        // 但同一条命令重复调用仍会被正常拦下
        assert_eq!(
            throttle_key("run_script", &json!({ "command": "cargo build" })),
            throttle_key("run_script", &json!({ "command": "cargo build" }))
        );
        // 完全取不到参数时才回落通配
        assert_eq!(throttle_key("list_files", &json!({})), "list_files|*");
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