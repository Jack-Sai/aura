use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use reqwest::StatusCode;
use serde_json::Value;

const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";

pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub context_limit: usize,
}

pub const MODELS: &[Model] = &[
    Model {
        id: "nvidia/nemotron-3-ultra-550b-a55b:free",
        label: "Ultra-550b",
        context_limit: 131072,
    },
    Model {
        id: "nvidia/nemotron-3-super-120b-a12b:free",
        label: "Super-120b",
        context_limit: 131072,
    },
    Model {
        id: "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free",
        label: "Nano-Omni-30b",
        context_limit: 65536,
    },
    Model {
        id: "nvidia/nemotron-3.5-lightning:free",
        label: "Lightning",
        context_limit: 131072,
    },
];

#[derive(Debug)]
pub enum ApiError {
    RateLimited,
    ServerError(u16),
    Timeout,
    Network(String),
    Auth(String),
    BadResponse(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::RateLimited => write!(f, "429 rate limited"),
            ApiError::ServerError(code) => write!(f, "server error {}", code),
            ApiError::Timeout => write!(f, "request timeout"),
            ApiError::Network(msg) => write!(f, "network error: {}", msg),
            ApiError::Auth(msg) => write!(f, "auth error: {}", msg),
            ApiError::BadResponse(msg) => write!(f, "bad response: {}", msg),
        }
    }
}

impl ApiError {
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            ApiError::RateLimited | ApiError::ServerError(_) | ApiError::Timeout | ApiError::Network(_)
        )
    }
}

fn classify_status(status: StatusCode, body: String) -> ApiError {
    let body = body.chars().take(500).collect::<String>();
    match status.as_u16() {
        429 => ApiError::RateLimited,
        code @ 500..=599 => ApiError::ServerError(code),
        401 | 403 => ApiError::Auth(body),
        _ => ApiError::BadResponse(format!("{} {}", status, body)),
    }
}

fn classify_reqwest(err: reqwest::Error) -> ApiError {
    if err.is_timeout() {
        ApiError::Timeout
    } else {
        ApiError::Network(err.to_string())
    }
}

/// 增量解析 SSE 行，提取 delta.content 文本
pub struct SseParser {
    buf: Vec<u8>,
    out: VecDeque<String>,
    finished: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            out: VecDeque::new(),
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
        if data == "[DONE]" {
            self.finished = true;
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let text = value
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str());
        if let Some(text) = text {
            if !text.is_empty() {
                self.out.push_back(text.to_string());
            }
        }
    }

    pub fn next(&mut self) -> Option<String> {
        self.out.pop_front()
    }

    pub fn is_finished(&self) -> bool {
        self.finished && self.out.is_empty()
    }
}

pub struct ChatStream {
    inner: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    parser: SseParser,
    pending: Option<ApiError>,
    done: bool,
}

impl ChatStream {
    fn new(resp: reqwest::Response) -> Self {
        Self {
            inner: Box::pin(resp.bytes_stream()),
            parser: SseParser::new(),
            pending: None,
            done: false,
        }
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

pub struct OpenRouterClient {
    http: reqwest::Client,
    api_key: Option<String>,
}

impl OpenRouterClient {
    pub fn new(api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build http client");
        Self { http, api_key }
    }

    fn request(&self, model: &str, messages: &[Value], stream: bool) -> reqwest::RequestBuilder {
        let mut req = self.http
            .post(ENDPOINT)
            .header("X-Title", "Aura")
            .json(&serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": stream,
            }));
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        req
    }

    pub async fn stream(&self, model: &str, messages: &[Value]) -> Result<ChatStream, ApiError> {
        let resp = self
            .request(model, messages, true)
            .send()
            .await
            .map_err(classify_reqwest)?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, body));
        }
        Ok(ChatStream::new(resp))
    }

    pub async fn complete(&self, model: &str, messages: &[Value]) -> Result<String, ApiError> {
        let resp = self
            .request(model, messages, false)
            .send()
            .await
            .map_err(classify_reqwest)?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, body));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| ApiError::BadResponse(e.to_string()))?;
        body["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ApiError::BadResponse("missing choices[0].message.content".into()))
    }

    async fn with_fallback<T, F, Fut>(&self, start_idx: usize, f: F) -> Result<Fallback<T>, ApiError>
    where
        F: Fn(usize) -> Fut,
        Fut: std::future::Future<Output = Result<T, ApiError>>,
    {
        let start = start_idx.min(MODELS.len() - 1);
        let mut first_err: Option<ApiError> = None;
        for offset in 0..MODELS.len() {
            let idx = start + offset;
            match f(idx).await {
                Ok(value) => {
                    let notice = if idx != start {
                        let reason = match &first_err {
                            Some(ApiError::RateLimited) => "限流",
                            _ => "故障",
                        };
                        Some(format!(
                            "主模型{}，已自动降级至 {}",
                            reason, MODELS[idx].label
                        ))
                    } else {
                        None
                    };
                    return Ok(Fallback {
                        value,
                        model_idx: idx,
                        notice,
                    });
                }
                Err(err) if err.is_retriable() && offset + 1 < MODELS.len() => {
                    first_err.get_or_insert(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(first_err.unwrap_or_else(|| ApiError::BadResponse("model list empty".into())))
    }

    pub async fn stream_with_fallback(
        &self,
        start_idx: usize,
        messages: &[Value],
    ) -> Result<Fallback<ChatStream>, ApiError> {
        self.with_fallback(start_idx, |idx| self.stream(MODELS[idx].id, messages))
            .await
    }

    pub async fn complete_with_fallback(
        &self,
        start_idx: usize,
        messages: &[Value],
    ) -> Result<Fallback<String>, ApiError> {
        self.with_fallback(start_idx, |idx| self.complete(MODELS[idx].id, messages))
            .await
    }
}

pub struct Fallback<T> {
    pub value: T,
    pub model_idx: usize,
    pub notice: Option<String>,
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
