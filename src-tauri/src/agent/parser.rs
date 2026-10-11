/// 用切片而非定长数组：新增标签时不必同步改长度，漏改会编译失败。
const TAGS: &[&str] = &[
    "<tool>",
    "</tool>",
    "<answer>",
    "</answer>",
    "<todo_plan>",
    "</todo_plan>",
    "<todo_update>",
    "</todo_update>",
];

const CLOSE_TOOL: &str = "</tool>";
const CLOSE_ANSWER: &str = "</answer>";
const CLOSE_PLAN: &str = "</todo_plan>";
const CLOSE_UPDATE: &str = "</todo_update>";

pub enum Event {
    Chunk(String),
    Tool(String),
    AnswerDone,
    /// `<todo_plan>` 的 JSON 载荷（不含标签本身）。
    ///
    /// 这是**伴随式**事件：与 `<tool>`/`<answer>` 不同，它不打断流。
    /// 计划写完之后模型还要继续输出 `<answer>` 做说明，
    /// 若在此 break 'stream，后续正文会整个丢失。
    TodoPlan(String),
    /// `<todo_update>` 的 JSON 载荷，形如 `{"id":"s1","status":"done"}`。
    /// 同样是伴随式事件，且会出现在流式输出的**任意位置**——
    /// 执行模式的正常形态就是「更新一步 → 调工具 → 更新一步 → 调工具」。
    TodoUpdate(String),
}

pub enum Finish {
    Ok,
    MissingTag,
    UnclosedTool,
    UnclosedPlan,
    UnclosedTodo,
}

enum State {
    Raw,
    Probe(String),
    Tool,
    Answer,
    Plan,
    Update,
}

enum Probe {
    NeedMore,
    Match(&'static str),
    Mismatch,
}

fn classify_probe(buf: &str) -> Probe {
    if let Some(tag) = TAGS.iter().find(|t| **t == buf) {
        return Probe::Match(tag);
    }
    if TAGS.iter().any(|t| t.starts_with(buf)) {
        return Probe::NeedMore;
    }
    Probe::Mismatch
}

fn safe_flush_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let max = (CLOSE_ANSWER.len() - 1).min(s.len());
    for k in (1..=max).rev() {
        if bytes[s.len() - k..] == CLOSE_ANSWER.as_bytes()[..k] {
            return s.len() - k;
        }
    }
    s.len()
}

pub struct Parser {
    state: State,
    tool_buf: String,
    ans_buf: String,
    plan_buf: String,
    upd_buf: String,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Raw,
            tool_buf: String::new(),
            ans_buf: String::new(),
            plan_buf: String::new(),
            upd_buf: String::new(),
        }
    }

    pub fn feed(&mut self, input: &str, out: &mut Vec<Event>) {
        let mut rest = input.to_string();
        while !rest.is_empty() {
            match self.state {
                State::Raw => match rest.find('<') {
                    None => out.push(Event::Chunk(std::mem::take(&mut rest))),
                    Some(pos) => {
                        let tail: String = rest.drain(pos..).collect();
                        if !rest.is_empty() {
                            out.push(Event::Chunk(std::mem::take(&mut rest)));
                        }
                        self.state = State::Probe(String::from("<"));
                        rest = tail[1..].to_string();
                    }
                },
                State::Probe(ref mut buf) => {
                    let take_len = match rest.find('>') {
                        Some(pos) => pos + 1,
                        None => rest.len(),
                    };
                    buf.push_str(&rest[..take_len]);
                    rest = rest[take_len..].to_string();
                    match classify_probe(buf) {
                        Probe::NeedMore => return,
                        Probe::Match(tag) => {
                            let _ = std::mem::take(buf);
                            match tag {
                                "<tool>" => {
                                    self.tool_buf.clear();
                                    self.state = State::Tool;
                                }
                                "<answer>" => {
                                    self.ans_buf.clear();
                                    self.state = State::Answer;
                                }
                                "<todo_plan>" => {
                                    self.plan_buf.clear();
                                    self.state = State::Plan;
                                }
                                "<todo_update>" => {
                                    self.upd_buf.clear();
                                    self.state = State::Update;
                                }
                                // 其余为关闭标签：落到这里说明是空闭合
                                // （如 `</answer>`），按原文继续流式输出
                                _ => self.state = State::Raw,
                            }
                        }
                        Probe::Mismatch => {
                            let buf = std::mem::take(buf);
                            self.state = State::Raw;
                            out.push(Event::Chunk(String::from("<")));
                            rest = buf[1..].to_string() + &rest;
                        }
                    }
                }
                State::Tool => {
                    self.tool_buf.push_str(&rest);
                    rest.clear();
                    if let Some(pos) = self.tool_buf.find(CLOSE_TOOL) {
                        let json: String = self.tool_buf.drain(..pos).collect();
                        self.tool_buf.drain(..CLOSE_TOOL.len());
                        out.push(Event::Tool(json.trim().to_string()));
                        self.state = State::Raw;
                        rest = std::mem::take(&mut self.tool_buf);
                    }
                }
State::Plan => {
                    // 计划体量小（几条标题），照 Tool 的做法整块吞下再解析，
                    // 不做流式预吐，避免把 JSON 一半一半地推给前端
                    self.plan_buf.push_str(&rest);
                    rest.clear();
                    if let Some(pos) = self.plan_buf.find(CLOSE_PLAN) {
                        let json: String = self.plan_buf.drain(..pos).collect();
                        self.plan_buf.drain(..CLOSE_PLAN.len());
                        let trimmed = json.trim().to_string();
                        if !trimmed.is_empty() {
                            out.push(Event::TodoPlan(trimmed));
                        }
                        self.state = State::Raw;
                        rest = std::mem::take(&mut self.plan_buf);
                    }
                }
State::Update => {
                    // 体量极小（一个 step_id + status），照 Plan/Plan 的做法整块吞下
                    self.upd_buf.push_str(&rest);
                    rest.clear();
                    if let Some(pos) = self.upd_buf.find(CLOSE_UPDATE) {
                        let json: String = self.upd_buf.drain(..pos).collect();
                        self.upd_buf.drain(..CLOSE_UPDATE.len());
                        let trimmed = json.trim().to_string();
                        if !trimmed.is_empty() {
                            out.push(Event::TodoUpdate(trimmed));
                        }
                        self.state = State::Raw;
                        rest = std::mem::take(&mut self.upd_buf);
                    }
                }
                State::Answer => {
                    self.ans_buf.push_str(&rest);
                    rest.clear();
                    match self.ans_buf.find(CLOSE_ANSWER) {
                        Some(pos) => {
                            let text: String = self.ans_buf.drain(..pos).collect();
                            self.ans_buf.drain(..CLOSE_ANSWER.len());
                            if !text.is_empty() {
                                out.push(Event::Chunk(text));
                            }
                            out.push(Event::AnswerDone);
                            self.state = State::Raw;
                            rest = std::mem::take(&mut self.ans_buf);
                        }
                        None => {
                            let flush = safe_flush_len(&self.ans_buf);
                            if flush > 0 {
                                let text: String = self.ans_buf.drain(..flush).collect();
                                out.push(Event::Chunk(text));
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    pub fn finish(&mut self, out: &mut Vec<Event>) -> Finish {
        match std::mem::replace(&mut self.state, State::Raw) {
            State::Answer => {
                if !self.ans_buf.is_empty() {
                    out.push(Event::Chunk(std::mem::take(&mut self.ans_buf)));
                }
                out.push(Event::AnswerDone);
                Finish::Ok
            }
            State::Tool => {
                let buf = std::mem::take(&mut self.tool_buf);
                if !buf.trim().is_empty() {
                    out.push(Event::Tool(buf.trim().to_string()));
                }
                Finish::UnclosedTool
            }
            State::Plan => {
                // 模型输出到一半断流：把已收到的内容照常抛出，
                // 让上层有机会解析出部分计划，而不是整个丢弃
                let buf = std::mem::take(&mut self.plan_buf);
                let trimmed = buf.trim().to_string();
                if !trimmed.is_empty() {
                    out.push(Event::TodoPlan(trimmed));
                }
                Finish::UnclosedPlan
            }
            State::Update => {
                let buf = std::mem::take(&mut self.upd_buf);
                let trimmed = buf.trim().to_string();
                if !trimmed.is_empty() {
                    out.push(Event::TodoUpdate(trimmed));
                }
                Finish::UnclosedTodo
            }
            State::Probe(buf) => {
                out.push(Event::Chunk(buf));
                Finish::MissingTag
            }
            State::Raw => Finish::MissingTag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(parts: &[&str]) -> (Vec<Event>, Finish) {
        let mut parser = Parser::new();
        let mut events = Vec::new();
        for part in parts {
            parser.feed(part, &mut events);
        }
        let finish = parser.finish(&mut events);
        (events, finish)
    }

    #[test]
    fn bare_text_then_tool() {
        let (events, _) = feed_all(&["我先看看", "<tool>{\"name\":\"list_files\"}</tool>"]);
        assert!(matches!(&events[0], Event::Chunk(t) if t == "我先看看"));
        assert!(matches!(&events[1], Event::Tool(t) if t == "{\"name\":\"list_files\"}"));
    }

    #[test]
    fn tag_split_across_chunks() {
        let (events, _) = feed_all(&["<to", "ol>{\"a\":", "1}</tool>rest"]);
        assert!(matches!(&events[0], Event::Tool(t) if t == "{\"a\":1}"));
        assert!(matches!(&events[1], Event::Chunk(t) if t == "rest"));
    }

    #[test]
    fn answer_streams_and_completes() {
        let (events, _) = feed_all(&["思考", "<answer>**你好**", "，世界</answer>多余"]);
        assert!(matches!(&events[0], Event::Chunk(t) if t == "思考"));
        assert!(matches!(&events[1], Event::Chunk(t) if t == "**你好**"));
        assert!(matches!(&events[2], Event::Chunk(t) if t == "，世界"));
        assert!(matches!(events[3], Event::AnswerDone));
        assert!(matches!(&events[4], Event::Chunk(t) if t == "多余"));
    }

    #[test]
    fn answer_close_tag_split_keeps_tail() {
        let (events, _) = feed_all(&["<answer>正文</ans", "wer>"]);
        assert!(matches!(&events[0], Event::Chunk(t) if t == "正文"));
        assert!(matches!(events[1], Event::AnswerDone));
    }

    #[test]
    fn lt_in_plain_text_is_not_tag() {
        let (events, finish) = feed_all(&["a < b"]);
        assert!(matches!(&events[0], Event::Chunk(t) if t == "a "));
        assert!(matches!(&events[1], Event::Chunk(t) if t == "<"));
        assert!(matches!(&events[2], Event::Chunk(t) if t == " b"));
        assert!(matches!(finish, Finish::MissingTag));
    }

    #[test]
    fn missing_tag_reported() {
        let (_, finish) = feed_all(&["只是思考，没有标签"]);
        assert!(matches!(finish, Finish::MissingTag));
    }

    #[test]
    fn unclosed_tool_reported() {
        let (events, finish) = feed_all(&["<tool>{\"x\":1}"]);
        assert!(matches!(&events[0], Event::Tool(t) if t == "{\"x\":1}"));
        assert!(matches!(finish, Finish::UnclosedTool));
    }

    #[test]
    fn todo_plan_is_emitted_without_breaking_the_stream() {
        // 计划之后模型还要输出 <answer> 做说明，
        // 因此 TodoPlan 事件后必须继续产出 Chunk 与 AnswerDone
        let (events, _) = feed_all(&[
            "调研完毕。",
            "<todo_plan>{\"todos\":[{\"id\":\"s1\",\"title\":\"建表\"}]}</todo_plan>",
            "接下来按计划推进。",
            "<answer>计划已给出</answer>",
        ]);
        assert!(matches!(&events[0], Event::Chunk(t) if t == "调研完毕。"));
        assert!(
            matches!(&events[1], Event::TodoPlan(t) if t.contains("\"id\":\"s1\"")),
            "应产出 TodoPlan 事件"
        );
        assert!(
            events.iter().any(|e| matches!(e, Event::Chunk(t) if t == "接下来按计划推进。")),
            "TodoPlan 之后必须继续输出正文，不能打断流"
        );
        assert!(events.iter().any(|e| matches!(e, Event::AnswerDone)));
    }

    #[test]
    fn todo_plan_tag_split_across_chunks() {
        let (events, _) = feed_all(&["<todo_p", "lan>{\"todos\":[]}</todo_", "plan>尾"]);
        assert!(matches!(&events[0], Event::TodoPlan(t) if t == "{\"todos\":[]}"));
        assert!(matches!(&events[1], Event::Chunk(t) if t == "尾"));
    }

    #[test]
    fn todo_plan_lt_in_text_is_not_a_tag() {
        // "<todo y" 不是合法标签，原文必须原样还原，不能被吞进计划缓冲区。
        // 断言拼接结果而非单个 chunk——"<" 会作为独立 chunk 先吐出。
        let (events, finish) = feed_all(&["x <todo y"]);
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                Event::Chunk(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "x <todo y");
        assert!(!events.iter().any(|e| matches!(e, Event::TodoPlan(_))));
        assert!(matches!(finish, Finish::MissingTag));
    }

    #[test]
    fn unclosed_plan_yields_partial_payload() {
        // 断流时把已收到的内容抛出，上层仍有机会解析出部分计划
        let (events, finish) = feed_all(&["<todo_plan>{\"todos\":[{\"title\":\"甲\"}]"]);
        assert!(matches!(&events[0], Event::TodoPlan(t) if t.contains("甲")));
        assert!(matches!(finish, Finish::UnclosedPlan));
    }

    #[test]
    fn empty_plan_tag_emits_nothing() {
        let (events, _) = feed_all(&["<todo_plan></todo_plan>x"]);
        assert!(!events.iter().any(|e| matches!(e, Event::TodoPlan(_))));
        assert!(events.iter().any(|e| matches!(e, Event::Chunk(t) if t == "x")));
    }

    #[test]
    fn todo_plan_coexists_with_tool_calls() {
        // 先给计划再执行，事件顺序不应互相吞掉
        let (events, _) = feed_all(&[
            "<todo_plan>{\"todos\":[{\"id\":\"s1\",\"title\":\"查一下\"}]}</todo_plan>",
            "<tool>{\"name\":\"list_files\"}</tool>",
        ]);
        assert!(events.iter().any(|e| matches!(e, Event::TodoPlan(_))));
        assert!(matches!(&events[1], Event::Tool(t) if t.contains("list_files")));
    }

    #[test]
    fn todo_update_tag_emits_and_keeps_streaming() {
        // Build 模式的正常形态：更新一步 → 调工具 → 更新一步
        let (events, _) = feed_all(&[
            "<todo_update>{\"id\":\"s1\",\"status\":\"in_progress\"}</todo_update>",
            "好的，开始处理。",
            "<tool>{\"name\":\"read_file\"}</tool>",
        ]);
        assert!(matches!(&events[0], Event::TodoUpdate(t) if t.contains("\"s1\"")));
        // 关键：更新之后正文仍要正常流出，不能像 <tool>/<answer> 那样打断
        assert!(
            events.iter().any(|e| matches!(e, Event::Chunk(t) if t.contains("好的"))),
            "TodoUpdate 后必须继续输出正文"
        );
        assert!(events.iter().any(|e| matches!(e, Event::Tool(_))));
    }

    #[test]
    fn multiple_todo_updates_in_one_stream() {
        let (events, _) = feed_all(&[
            "<todo_update>{\"id\":\"s1\",\"status\":\"in_progress\"}</todo_update>",
            "<todo_update>{\"id\":\"s1\",\"status\":\"done\"}</todo_update>",
            "<todo_update>{\"id\":\"s2\",\"status\":\"done\"}</todo_update>",
        ]);
        let updates: Vec<&Event> =
            events.iter().filter(|e| matches!(e, Event::TodoUpdate(_))).collect();
        assert_eq!(updates.len(), 3);
        assert!(matches!(updates[0], Event::TodoUpdate(t) if t.contains("in_progress")));
        assert!(matches!(updates[2], Event::TodoUpdate(t) if t.contains("\"s2\"")));
    }

    #[test]
    fn todo_update_split_across_chunks() {
        let (events, _) = feed_all(&["<todo_up", "date>{\"id\":\"s1\"}</todo_", "update>尾"]);
        assert!(matches!(&events[0], Event::TodoUpdate(t) if t == "{\"id\":\"s1\"}"));
        assert!(matches!(&events[1], Event::Chunk(t) if t == "尾"));
    }

    #[test]
    fn todo_update_lt_in_text_is_not_a_tag() {
        let (events, finish) = feed_all(&["x <todo_update y"]);
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                Event::Chunk(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "x <todo_update y");
        assert!(!events.iter().any(|e| matches!(e, Event::TodoUpdate(_))));
        assert!(matches!(finish, Finish::MissingTag));
    }

    #[test]
    fn unclosed_todo_update_yields_partial_payload() {
        let (events, finish) = feed_all(&["<todo_update>{\"id\":\"s1\",\"stat"]);
        assert!(matches!(&events[0], Event::TodoUpdate(t) if t.contains("s1")));
        assert!(matches!(finish, Finish::UnclosedTodo));
    }

    #[test]
    fn empty_todo_update_emits_nothing() {
        let (events, _) = feed_all(&["<todo_update></todo_update>尾"]);
        assert!(!events.iter().any(|e| matches!(e, Event::TodoUpdate(_))));
        assert!(events.iter().any(|e| matches!(e, Event::Chunk(t) if t == "尾")));
    }

    #[test]
    fn todo_plan_and_update_tags_do_not_confuse_each_other() {
        // 两种标签前缀相近，最容易在 Probe 状态机里串味
        let (events, _) = feed_all(&[
            "<todo_plan>{\"todos\":[]}</todo_plan>",
            "<todo_update>{\"id\":\"s1\"}</todo_update>",
        ]);
        assert!(matches!(&events[0], Event::TodoPlan(_)));
        assert!(matches!(&events[1], Event::TodoUpdate(_)));
    }

    /// 跨模块：parser 产出的 TodoPlan 载荷必须能被 `todo::parse_plan` 直接消费。
    ///
    /// parser 只负责切标签，不校验 JSON。两段各自测试都通过，
    /// 但若某段擅自改了载荷格式（例如带上标签本身），
    /// 组合起来就会在运行时炸——只有串起来才暴露。
    #[test]
    fn parser_plan_payload_is_accepted_by_todo_parser() {
        let (events, _) = feed_all(&[
            "<todo_plan>{\"todos\":[{\"id\":\"s1\",\"title\":\"建表\"},{\"id\":\"s2\",\"title\":\"写测试\"}]}</todo_plan>",
        ]);
        let raw = events
            .iter()
            .find_map(|e| match e {
                Event::TodoPlan(t) => Some(t.clone()),
                _ => None,
            })
            .expect("应产出 TodoPlan 事件");

        let parsed = crate::agent::todo::parse_plan(&raw).expect("载荷应能被 todo 解析");
        assert_eq!(parsed.todos.len(), 2);
        assert_eq!(parsed.todos[0].id, "s1");
    }

    /// 跨模块：parser 产出的 TodoUpdate 载荷必须能驱动清单状态变更。
    #[test]
    fn parser_update_payload_drives_todo_state() {
        let mut todos =
            crate::agent::todo::parse_plan(r#"[{"id":"s1","title":"建表"}]"#).unwrap().todos;

        let (events, _) = feed_all(&[
            "<todo_update>{\"id\":\"s1\",\"status\":\"in_progress\"}</todo_update>",
            "开始处理",
            "<todo_update>{\"id\":\"s1\",\"status\":\"done\"}</todo_update>",
        ]);

        for ev in &events {
            if let Event::TodoUpdate(raw) = ev {
                let v: serde_json::Value = serde_json::from_str(raw).expect("应是合法 JSON");
                let id = v.get("id").and_then(|x| x.as_str()).unwrap();
                let status =
                    crate::agent::TodoStatus::parse(v.get("status").and_then(|x| x.as_str()).unwrap());
                crate::agent::todo::apply_update(&mut todos, id, status);
            }
        }

        assert_eq!(todos[0].status, crate::agent::TodoStatus::Done);
        assert!(crate::agent::todo::done_count(&todos) == 1);
    }

    #[test]
    fn unclosed_answer_treated_as_done() {
        let (events, finish) = feed_all(&["<answer>答案没写完"]);
        assert!(matches!(&events[0], Event::Chunk(t) if t == "答案没写完"));
        assert!(matches!(events[1], Event::AnswerDone));
        assert!(matches!(finish, Finish::Ok));
    }

    #[test]
    fn chinese_utf8_safe_flush() {
        let mut parser = Parser::new();
        let mut events = Vec::new();
        parser.feed("<answer>中文内容里出现结束标记的前", &mut events);
        assert!(matches!(&events[0], Event::Chunk(t) if t == "中文内容里出现结束标记的前"));
        parser.feed("缀</answer>", &mut events);
        assert!(matches!(&events[1], Event::Chunk(t) if t == "缀"));
        assert!(matches!(events[2], Event::AnswerDone));
    }
}
