//! 任务清单（v0.4.0）
//!
//! 规划模式产出的「要做哪些事」，以及执行模式推进到哪一步。
//!
//! 权威态存在 `sessions.todos_json` 列，而非对话消息里。原因是：
//!
//! - [`crate::db::save_session`] 是 DELETE+INSERT 全量重写所有消息，
//!   清单每更新一次就重写整段会话，长对话下写放大明显；
//! - 清单需要支持用户手动编辑，塞进消息历史语义上很别扭；
//! - 上下文压缩只作用于 agent 历史，清单独立存储天然不受影响。

use serde::{Deserialize, Serialize};

/// 单个步骤的状态。
///
/// 序列化值必须与 `as_str` 一致，因此逐个显式指定而非用 `rename_all`——
/// `lowercase` 会把 `InProgress` 变成 `inprogress`，与对外契约的
/// `in_progress` 不符，导致落盘后前端读不回状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TodoStatus {
    #[default]
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "in_progress")]
    InProgress,
    #[serde(rename = "done")]
    Done,
}

impl TodoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Done => "done",
        }
    }

    /// 解析模型输出或前端传入的状态串，无法识别时回落 `pending`。
    ///
    /// 模型偶尔会输出 `completed`/`finished` 之类的近义词，这里做宽松匹配
    /// 而不是直接拒绝——一步被误标成 pending 的代价，远小于丢整份清单。
    pub fn parse(raw: &str) -> TodoStatus {
        match raw.trim().to_ascii_lowercase().as_str() {
            "in_progress" | "inprogress" | "in-progress" | "doing" | "active" => {
                TodoStatus::InProgress
            }
            "done" | "completed" | "complete" | "finished" | "finished_" => TodoStatus::Done,
            _ => TodoStatus::Pending,
        }
    }

    /// 排序权重：进行中优先于未开始，已完成沉底。用于面板展示顺序。
    fn rank(self) -> u8 {
        match self {
            TodoStatus::InProgress => 0,
            TodoStatus::Pending => 1,
            TodoStatus::Done => 2,
        }
    }
}

/// 一个任务步骤。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    /// 步骤标识，约定用 s1、s2 递增。清单内唯一。
    pub id: String,
    /// 步骤描述，应具体到可验证。
    pub title: String,
    #[serde(default)]
    pub status: TodoStatus,
}

impl Todo {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Todo {
        Todo {
            id: id.into(),
            title: title.into(),
            status: TodoStatus::Pending,
        }
    }

    pub fn is_done(&self) -> bool {
        self.status == TodoStatus::Done
    }
}

/// 一份完整清单的校验与规范化结果。
pub struct ParsedPlan {
    pub todos: Vec<Todo>,
    /// 被丢弃的条目数。模型偶尔会漏字段或给重复 id，静默丢弃会让人
    /// 以为清单完整，因此这里把丢弃数量显式带出来，由调用方回传给用户。
    pub dropped: usize,
}

/// 解析模型产出的 `<todo_plan>` JSON。
///
/// 容错是有意为之：模型输出不是可信输入，宁可丢几条也不能让整份清单
/// 解析失败——否则用户看到的是「模型没给出计划」，而不是「计划里少两条」。
///
/// 接收 `{"todos":[...]}` 或裸数组 `[{...}]` 两种形态。
pub fn parse_plan(raw: &str) -> Result<ParsedPlan, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw.trim()).map_err(|e| format!("JSON parse failed - {e}"))?;

    let items = match &value {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Object(o) => o
            .get("todos")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| "missing field `todos`".to_string())?,
        _ => return Err("expected array or object with `todos`".to_string()),
    };

    let mut todos: Vec<Todo> = Vec::new();
    let mut dropped = 0usize;
    for (idx, item) in items.iter().enumerate() {
        // 标题是唯一必需字段：没有标题的步骤对用户毫无意义
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(title) = title else {
            dropped += 1;
            continue;
        };
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("s{}", idx + 1));
        // 重复 id 会让 todo_update 无法定位步骤，后出现的直接丢弃
        if todos.iter().any(|t| t.id == id) {
            dropped += 1;
            continue;
        }
        let status = item
            .get("status")
            .and_then(|v| v.as_str())
            .map(TodoStatus::parse)
            .unwrap_or_default();
        todos.push(Todo { id, title: title.to_string(), status });
    }

    if todos.is_empty() {
        return Err("plan contains no valid steps".to_string());
    }
    Ok(ParsedPlan { todos, dropped })
}

/// 将单个 `todo_update` 应用到清单上，返回是否命中。
///
/// 未命中不是错误：模型可能引用一个已被用户删掉的步骤，静默忽略即可。
pub fn apply_update(todos: &mut [Todo], id: &str, status: TodoStatus) -> bool {
    match todos.iter_mut().find(|t| t.id == id) {
        Some(todo) => {
            todo.status = status;
            true
        }
        None => false,
    }
}

/// 校验并覆盖式保存前端提交的清单。
///
/// 与 agent 产出的清单走同一套规范化规则：去空标题、去重复 id。
/// 手动编辑和模型生成必须共享不变量，否则两侧会产出结构不同的清单，
/// 让后续的 `todo_update` 在某些会话里莫名其妙失配。
pub fn sanitize(input: Vec<Todo>) -> (Vec<Todo>, usize) {
    let mut todos: Vec<Todo> = Vec::new();
    let mut dropped = 0usize;
    for t in input {
        let title = t.title.trim();
        let id = t.id.trim();
        if title.is_empty() || id.is_empty() {
            dropped += 1;
            continue;
        }
        if todos.iter().any(|x| x.id == id) {
            dropped += 1;
            continue;
        }
        todos.push(Todo {
            id: id.to_string(),
            title: title.to_string(),
            status: t.status,
        });
    }
    (todos, dropped)
}

/// 面板展示顺序：进行中在前，未开始居中，已完成沉底。
///
/// 同组内保持原顺序——用户手动排的顺序是有意义的，不应被重排打乱。
pub fn display_order(todos: &mut Vec<Todo>) {
    todos.sort_by_key(|t| t.status.rank());
}

/// 已完成步骤数。
pub fn done_count(todos: &[Todo]) -> usize {
    todos.iter().filter(|t| t.is_done()).count()
}

/// 渲染成注入 System Prompt 的紧凑文本。
///
/// 为空时返回空串，调用方据此跳过该段落，避免 prompt 里出现空标题。
pub fn render_for_prompt(todos: &[Todo]) -> String {
    if todos.is_empty() {
        return String::new();
    }
    let mut s = format!("\n# 当前任务清单（{}/{} 已完成）\n", done_count(todos), todos.len());
    for t in todos {
        let mark = match t.status {
            TodoStatus::Done => "x",
            TodoStatus::InProgress => ">",
            TodoStatus::Pending => " ",
        };
        s.push_str(&format!("- [{}] {}: {}\n", mark, t.id, t.title));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(json: &str) -> Vec<Todo> {
        parse_plan(json).expect("应当解析成功").todos
    }

    #[test]
    fn parses_object_with_todos_key() {
        let todos = plan(r#"{"todos":[{"id":"s1","title":"建表"},{"id":"s2","title":"写测试"}]}"#);
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].id, "s1");
        assert_eq!(todos[1].title, "写测试");
        assert!(todos.iter().all(|t| t.status == TodoStatus::Pending));
    }

    #[test]
    fn parses_bare_array() {
        // 模型常直接给数组而非包一层对象，两种都要收
        let todos = plan(r#"[{"id":"s1","title":"只做一件事"}]"#);
        assert_eq!(todos.len(), 1);
    }

    #[test]
    fn missing_title_is_dropped_and_counted() {
        let parsed = parse_plan(
            r#"{"todos":[{"id":"s1","title":"保留"},{"id":"s2"},{"title":"缺 id 也要保留"}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.todos.len(), 2);
        assert_eq!(parsed.dropped, 1);
        assert_eq!(parsed.todos[1].title, "缺 id 也要保留");
    }

    #[test]
    fn missing_id_is_generated_by_position() {
        let todos = plan(r#"[{"title":"甲"},{"title":"乙"}]"#);
        assert_eq!(todos[0].id, "s1");
        assert_eq!(todos[1].id, "s2");
    }

    #[test]
    fn duplicate_ids_keep_the_first() {
        // 重复 id 会让后续 todo_update 无法定位步骤，必须去重
        let parsed =
            parse_plan(r#"[{"id":"s1","title":"先来"},{"id":"s1","title":"后来的丢弃"}]"#).unwrap();
        assert_eq!(parsed.todos.len(), 1);
        assert_eq!(parsed.todos[0].title, "先来");
        assert_eq!(parsed.dropped, 1);
    }

    #[test]
    fn entirely_invalid_plan_is_an_error() {
        assert!(parse_plan(r#"{"todos":[]}"#).is_err());
        assert!(parse_plan("not json").is_err());
        assert!(parse_plan(r#"{"other":1}"#).is_err());
    }

    #[test]
    fn status_parsing_accepts_synonyms() {
        assert_eq!(TodoStatus::parse("in_progress"), TodoStatus::InProgress);
        assert_eq!(TodoStatus::parse("DONE"), TodoStatus::Done);
        assert_eq!(TodoStatus::parse("completed"), TodoStatus::Done);
        // 未知值回落 pending 而非报错
        assert_eq!(TodoStatus::parse("weird"), TodoStatus::Pending);
    }

    #[test]
    fn apply_update_hits_and_misses() {
        let mut todos = plan(r#"[{"id":"s1","title":"甲"},{"id":"s2","title":"乙"}]"#);
        assert!(apply_update(&mut todos, "s1", TodoStatus::Done));
        assert_eq!(todos[0].status, TodoStatus::Done);
        // 引用不存在的步骤不算错误，静默忽略
        assert!(!apply_update(&mut todos, "nope", TodoStatus::Done));
        assert_eq!(todos.len(), 2);
    }

    /// Build 模式的典型推进：pending → in_progress → done，
    /// 状态要能被反复覆盖，且只影响目标步骤。
    #[test]
    fn apply_update_transitions_through_full_lifecycle() {
        let mut todos = plan(r#"[{"id":"s1","title":"甲"},{"id":"s2","title":"乙"}]"#);
        assert!(apply_update(&mut todos, "s1", TodoStatus::InProgress));
        assert_eq!(todos[0].status, TodoStatus::InProgress);
        assert_eq!(todos[1].status, TodoStatus::Pending, "不应影响其它步骤");

        assert!(apply_update(&mut todos, "s1", TodoStatus::Done));
        assert_eq!(todos[0].status, TodoStatus::Done);

        // 已完成的步骤允许被回退（例如发现验证失败需要重做）
        assert!(apply_update(&mut todos, "s1", TodoStatus::InProgress));
        assert_eq!(todos[0].status, TodoStatus::InProgress);
        assert_eq!(done_count(&todos), 0);
    }

    #[test]
    fn sanitize_shares_rules_with_model_output() {
        let input = vec![
            Todo::new("s1", "  保留  "),
            Todo::new("", "空 id"),
            Todo::new("s1", "重复 id"),
            Todo::new("s4", "   "),
        ];
        let (todos, dropped) = sanitize(input);
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].title, "保留");
        assert_eq!(todos[0].id, "s1");
        assert_eq!(dropped, 3);
    }

    #[test]
    fn display_order_puts_active_first_and_done_last() {
        let mut todos = plan(
            r#"[{"id":"s1","title":"a","status":"done"},
                {"id":"s2","title":"b"},
                {"id":"s3","title":"c","status":"in_progress"}]"#,
        );
        display_order(&mut todos);
        assert_eq!(
            todos.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["s3", "s2", "s1"]
        );
    }

    #[test]
    fn display_order_is_stable_within_a_status() {
        // 用户手动排的顺序有意义，不能被排序打乱
        let mut todos = plan(
            r#"[{"id":"s1","title":"a"},{"id":"s2","title":"b"},{"id":"s3","title":"c"}]"#,
        );
        display_order(&mut todos);
        assert_eq!(
            todos.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["s1", "s2", "s3"]
        );
    }

    #[test]
    fn prompt_render_is_empty_when_no_todos() {
        assert_eq!(render_for_prompt(&[]), "");
    }

    #[test]
    fn prompt_render_shows_progress_and_marks() {
        let todos = plan(
            r#"[{"id":"s1","title":"建表","status":"done"},
                {"id":"s2","title":"写测试","status":"in_progress"},
                {"id":"s3","title":"接面板"}]"#,
        );
        let s = render_for_prompt(&todos);
        assert!(s.contains("1/3 已完成"));
        assert!(s.contains("- [x] s1: 建表"));
        assert!(s.contains("- [>] s2: 写测试"));
        assert!(s.contains("- [ ] s3: 接面板"));
    }

    #[test]
    fn status_round_trips_through_serde() {
        for st in [TodoStatus::Pending, TodoStatus::InProgress, TodoStatus::Done] {
            let json = serde_json::to_string(&st).unwrap();
            assert_eq!(json, format!("\"{}\"", st.as_str()));
            let back: TodoStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, st);
        }
    }
}