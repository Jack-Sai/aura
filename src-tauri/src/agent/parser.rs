/// 用切片而非定长数组：新增标签时不必同步改长度，漏改会编译失败。
const TAGS: &[&str] = &[
    "<tool>",
    "</tool>",
    "<answer>",
    "</answer>",
    "<todo_plan>",
    "</todo_plan>",
];

const CLOSE_TOOL: &str = "</tool>";
const CLOSE_ANSWER: &str = "</answer>";
const CLOSE_PLAN: &str = "</todo_plan>";

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
}

pub enum Finish {
    Ok,
    MissingTag,
    UnclosedTool,
    UnclosedPlan,
}

enum State {
    Raw,
    Probe(String),
    Tool,
    Answer,
    Plan,
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
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Raw,
            tool_buf: String::new(),
            ans_buf: String::new(),
            plan_buf: String::new(),
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
