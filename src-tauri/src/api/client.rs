use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use reqwest::StatusCode;
use serde_json::Value;

const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";

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

#[derive(Debug, Clone)]
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

/// 独立的模型供应商（Provider）结构体：封装 endpoint、鉴权与请求构造。
/// 当前仅实现 OpenRouter；v0.2.0 将在此之上抽取统一 Trait 抽象层。
pub struct OpenRouterProvider {
    name: &'static str,
    endpoint: String,
    http: reqwest::Client,
    api_key: std::sync::RwLock<Option<String>>,
}

pub type OpenRouterClient = OpenRouterProvider;

impl OpenRouterProvider {
    pub fn openrouter() -> Self {
        Self::with_endpoint("openrouter", OPENROUTER_ENDPOINT)
    }

    pub fn with_endpoint(name: &'static str, endpoint: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build http client");
        Self {
            name,
            endpoint: endpoint.to_string(),
            http,
            api_key: std::sync::RwLock::new(None),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.read().map(|k| k.is_some()).unwrap_or(false)
    }

    pub fn api_key(&self) -> Option<String> {
        self.api_key.read().ok().and_then(|g| g.clone())
    }

    pub fn set_api_key(&self, key: Option<String>) {
        if let Ok(mut g) = self.api_key.write() {
            *g = key;
        }
    }

    fn request(&self, model: &str, messages: &[Value], stream: bool) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .post(&self.endpoint)
            .header("X-Title", "Aura")
            .json(&serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": stream,
                "max_tokens": 16384,
            }));
        if let Ok(guard) = self.api_key.read() {
            if let Some(key) = guard.as_ref() {
                req = req.bearer_auth(key);
            }
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
        // 只在 start..len 内遍历，避免从末尾模型降级时越界
        let remaining = MODELS.len() - start;
        let mut first_err: Option<ApiError> = None;
        for offset in 0..remaining {
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
                Err(err) if err.is_retriable() && offset + 1 < remaining => {
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

#[derive(Debug)]
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

    #[test]
    fn provider_construction_and_api_key() {
        let p = OpenRouterProvider::openrouter();
        assert_eq!(p.name(), "openrouter");
        assert_eq!(p.endpoint(), OPENROUTER_ENDPOINT);
        assert!(!p.has_api_key());
        p.set_api_key(Some("sk-or-test".into()));
        assert!(p.has_api_key());
        assert_eq!(p.api_key().as_deref(), Some("sk-or-test"));
        p.set_api_key(None);
        assert!(!p.has_api_key());

        let custom = OpenRouterProvider::with_endpoint("custom", "http://localhost:8080/v1");
        assert_eq!(custom.endpoint(), "http://localhost:8080/v1");
        assert_eq!(custom.name(), "custom");
    }

    async fn run_fallback(
        start: usize,
        outcomes: Vec<Result<usize, ApiError>>,
    ) -> Result<Fallback<usize>, ApiError> {
        let p = OpenRouterProvider::openrouter();
        p.with_fallback(start, |idx| {
            let outcome = outcomes[idx].clone();
            async move { outcome }
        })
        .await
    }

    #[tokio::test]
    async fn fallback_success_on_first_model_has_no_notice() {
        let out = run_fallback(0, vec![Ok(0), Ok(1)]).await.unwrap();
        assert_eq!(out.model_idx, 0);
        assert!(out.notice.is_none());
    }

    #[tokio::test]
    async fn fallback_rate_limited_switches_to_next_and_notifies() {
        let out = run_fallback(0, vec![Err(ApiError::RateLimited), Ok(1), Ok(2)])
            .await
            .unwrap();
        assert_eq!(out.model_idx, 1);
        let n = out.notice.expect("应有降级提示");
        assert!(n.contains("限流"), "notice: {}", n);
        assert!(n.contains("降级"), "notice: {}", n);
    }

    #[tokio::test]
    async fn fallback_walks_chain_until_success() {
        let out = run_fallback(
            0,
            vec![
                Err(ApiError::RateLimited),
                Err(ApiError::ServerError(500)),
                Ok(2),
            ],
        )
        .await
        .unwrap();
        assert_eq!(out.model_idx, 2);
        assert!(out.notice.is_some());
    }

    #[tokio::test]
    async fn fallback_stops_on_auth_error_without_retry() {
        let err = run_fallback(0, vec![Err(ApiError::Auth("bad key".into())), Ok(1)])
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Auth(_)));
    }

    #[tokio::test]
    async fn fallback_all_failed_returns_first_retriable_error() {
        let err = run_fallback(
            1,
            vec![
                Ok(0),
                Err(ApiError::RateLimited),
                Err(ApiError::RateLimited),
                Err(ApiError::RateLimited),
            ],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::RateLimited));
    }

    #[tokio::test]
    async fn fallback_respects_start_index() {
        // 从 start_idx 开始遍历：idx0 的结果不应被使用
        let out = run_fallback(2, vec![Ok(0), Ok(1), Err(ApiError::Timeout), Ok(3)])
            .await
            .unwrap();
        assert_eq!(out.model_idx, 3);
        assert!(out.notice.is_some());
    }

    #[tokio::test]
    async fn fallback_at_last_model_does_not_panic() {
        // 选中最后一个模型时降级链必须收敛，不能越界
        let out = run_fallback(3, vec![Ok(0), Ok(1), Ok(2), Ok(3)]).await.unwrap();
        assert_eq!(out.model_idx, 3);
        assert!(out.notice.is_none());

        let err = run_fallback(
            3,
            vec![Ok(0), Ok(1), Ok(2), Err(ApiError::RateLimited)],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::RateLimited));
    }
}
