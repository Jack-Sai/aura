const TAGS: [&str; 4] = ["<tool>", "</tool>", "<answer>", "</answer>"];

const CLOSE_TOOL: &str = "</tool>";
const CLOSE_ANSWER: &str = "</answer>";

pub enum Event {
    Chunk(String),
    Tool(String),
    AnswerDone,
}

pub enum Finish {
    Ok,
    MissingTag,
    UnclosedTool,
}

enum State {
    Raw,
    Probe(String),
    Tool,
    Answer,
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
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Raw,
            tool_buf: String::new(),
            ans_buf: String::new(),
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
                self.tool_buf.clear();
                Finish::UnclosedTool
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
        assert!(events.is_empty());
        assert!(matches!(finish, Finish::UnclosedTool));
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
