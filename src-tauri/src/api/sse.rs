use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::error::{classify_reqwest, ApiError};
use futures_util::Stream;
use serde_json::Value;

/// 增量解析 SSE 行，提取 delta.content 文本
pub struct SseParser {
    buf: Vec<u8>,
    out: VecDeque<String>,
    reasoning: String,
    event_count: usize,
    raw_tail: String,
    finished: bool,
}

const RAW_TAIL_MAX: usize = 4000;

impl SseParser {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            out: VecDeque::new(),
            reasoning: String::new(),
            event_count: 0,
            raw_tail: String::new(),
            finished: false,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            self.handle_line(&line);
        }
    }

    fn handle_line(&mut self, raw: &str) {
        let line = raw.trim();
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim();
        self.event_count += 1;
        self.raw_tail.push_str(data);
        self.raw_tail.push('\n');
        if self.raw_tail.len() > RAW_TAIL_MAX {
            let cut = self.raw_tail.len() - RAW_TAIL_MAX;
            let mut chars = self.raw_tail.char_indices().skip_while(|(i, _)| *i < cut);
            if let Some((i, _)) = chars.next() {
                self.raw_tail.drain(..i);
            }
        }
        if data == "[DONE]" {
            self.finished = true;
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let delta = value
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"));
        let text = delta
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str());
        if let Some(text) = text {
            if !text.is_empty() {
                self.out.push_back(text.to_string());
            }
        }
        for key in ["reasoning", "reasoning_content"] {
            if let Some(r) = delta.and_then(|d| d.get(key)).and_then(|r| r.as_str()) {
                if !r.is_empty() {
                    self.reasoning.push_str(r);
                }
            }
        }
        if let Some(details) = delta.and_then(|d| d.get("reasoning_details")) {
            if let Some(arr) = details.as_array() {
                for item in arr {
                    if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                        if !t.is_empty() {
                            self.reasoning.push_str(t);
                        }
                    }
                }
            }
        }
    }

    pub fn next(&mut self) -> Option<String> {
        self.out.pop_front()
    }

    pub fn take_reasoning(&mut self) -> String {
        std::mem::take(&mut self.reasoning)
    }

    pub fn diag(&self) -> (usize, String) {
        (self.event_count, self.raw_tail.clone())
    }

    pub fn is_finished(&self) -> bool {
        self.finished && self.out.is_empty()
    }
}

/// OpenAI 兼容 SSE 响应流：按需拉取字节块，增量解析出文本。
pub struct ChatStream {
    inner: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    parser: SseParser,
    pending: Option<ApiError>,
    done: bool,
}

impl ChatStream {
    pub(super) fn new(resp: reqwest::Response) -> Self {
        Self {
            inner: Box::pin(resp.bytes_stream()),
            parser: SseParser::new(),
            pending: None,
            done: false,
        }
    }

    pub fn take_reasoning(&mut self) -> String {
        self.parser.take_reasoning()
    }

    pub fn diag(&self) -> (usize, String) {
        self.parser.diag()
    }
}

impl Stream for ChatStream {
    type Item = Result<String, ApiError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(text) = this.parser.next() {
                return Poll::Ready(Some(Ok(text)));
            }
            if let Some(err) = this.pending.take() {
                this.done = true;
                return Poll::Ready(Some(Err(err)));
            }
            if this.done || this.parser.is_finished() {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.parser.feed(&chunk);
                }
                Poll::Ready(Some(Err(err))) => {
                    this.pending = Some(classify_reqwest(err));
                }
                Poll::Ready(None) => {
                    this.done = true;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parses_split_chunks() {
        let mut parser = SseParser::new();
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n";
        let bytes = line.as_bytes();
        parser.feed(&bytes[..20]);
        assert_eq!(parser.next(), None);
        parser.feed(&bytes[20..]);
        assert_eq!(parser.next().as_deref(), Some("你好"));
    }

    #[test]
    fn sse_accumulates_reasoning() {
        let mut parser = SseParser::new();
        let input = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"思考一\"}}]}\n\
              data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"思考二\"}}]}\n\
              data: {\"choices\":[{\"delta\":{\"content\":\"答案\"}}]}\n";
        parser.feed(input.as_bytes());
        assert_eq!(parser.next().as_deref(), Some("答案"));
        assert_eq!(parser.take_reasoning(), "思考一思考二");
        assert_eq!(parser.take_reasoning(), "");
    }

    #[test]
    fn sse_extracts_reasoning_details_array() {
        let mut parser = SseParser::new();
        let input = "data: {\"choices\":[{\"delta\":{\"reasoning_details\":[\
            {\"type\":\"reasoning_text\",\"text\":\"详细思考\"}]}}]}\n";
        parser.feed(input.as_bytes());
        assert_eq!(parser.take_reasoning(), "详细思考");
        let (count, tail) = parser.diag();
        assert_eq!(count, 1);
        assert!(tail.contains("reasoning_details"));
    }

    #[test]
    fn sse_handles_done_and_empty() {
        let mut parser = SseParser::new();
        parser.feed(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: {\"choices\":[{\"delta\":{}}]}\ndata: [DONE]\n",
        );
        assert_eq!(parser.next().as_deref(), Some("a"));
        assert_eq!(parser.next(), None);
        assert!(parser.is_finished());
    }

    #[test]
    fn sse_ignores_unknown_lines() {
        let mut parser = SseParser::new();
        parser.feed(b": OPENROUTER PROCESSING\nevent: message\n{\"x\":1}\n");
        assert_eq!(parser.next(), None);
        assert!(!parser.is_finished());
    }
}
