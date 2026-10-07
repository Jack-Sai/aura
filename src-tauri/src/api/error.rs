use reqwest::StatusCode;

/// 模型后端统一错误枚举（v0.2.0 抽象层）。
/// 覆盖网络、限流、鉴权、上下文溢出、本地进程异常等所有 Provider 需要表达的错误类别。
#[derive(Debug, Clone)]
pub enum ModelError {
    /// 429 限流
    RateLimited,
    /// 5xx 服务端错误
    ServerError(u16),
    /// 请求超时
    Timeout,
    /// 网络不可达等传输层错误
    Network(String),
    /// 鉴权失败（401/403）
    Auth(String),
    /// 请求本身不合法（400/404/422 等）
    InvalidRequest(String),
    /// 上下文超出模型窗口
    ContextOverflow,
    /// 响应结构不符合预期
    BadResponse(String),
    /// 本地推理服务未启动/连接被拒
    LocalUnavailable(String),
    /// 当前 Provider 不支持该能力
    Unsupported(String),
    /// 链上的供应商未启用/执行器缺失（跳过并继续降级）
    ProviderMissing(String),
}

/// 兼容旧代码的类型别名，后续迁移完成后移除。
pub type ApiError = ModelError;

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::RateLimited => write!(f, "模型服务限流（429）"),
            ModelError::ServerError(code) => write!(f, "模型服务端错误（{}）", code),
            ModelError::Timeout => write!(f, "请求超时"),
            ModelError::Network(msg) => write!(f, "网络错误：{}", msg),
            ModelError::Auth(msg) => write!(f, "鉴权失败：{}", msg),
            ModelError::InvalidRequest(msg) => write!(f, "请求被拒绝：{}", msg),
            ModelError::ContextOverflow => write!(f, "对话超出模型上下文窗口"),
            ModelError::BadResponse(msg) => write!(f, "响应格式异常：{}", msg),
            ModelError::LocalUnavailable(msg) => write!(f, "本地模型服务不可用：{}", msg),
            ModelError::Unsupported(msg) => write!(f, "当前服务不支持：{}", msg),
            ModelError::ProviderMissing(msg) => write!(f, "模型供应商不可用：{}", msg),
        }
    }
}

impl ModelError {
    /// 是否值得切换到降级链上的下一个模型重试。
    /// ContextOverflow 可重试（链上可能存在更大窗口的模型）；
    /// Auth/InvalidRequest/BadResponse/Unsupported 属于确定性错误，不切换。
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            ModelError::RateLimited
                | ModelError::ServerError(_)
                | ModelError::Timeout
                | ModelError::Network(_)
                | ModelError::ContextOverflow
                | ModelError::LocalUnavailable(_)
                | ModelError::ProviderMissing(_)
        )
    }

    /// 是否属于鉴权类错误（UI 判断是否需要提示重新配置 API Key）。
    pub fn is_auth(&self) -> bool {
        matches!(self, ModelError::Auth(_))
    }
}

/// 上下文溢出的响应体特征（OpenAI 兼容各家文案不一，宽松匹配）。
fn looks_like_context_overflow(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("context length")
        || b.contains("context_length")
        || b.contains("maximum context")
        || b.contains("max context")
        || b.contains("context window")
        || b.contains("too many tokens")
        || b.contains("prompt is too long")
        || b.contains("exceeds the context")
        || b.contains("input length and `max_tokens`")
}

pub fn classify_status(status: StatusCode, body: String) -> ModelError {
    let body: String = body.chars().take(500).collect();
    match status.as_u16() {
        429 => ModelError::RateLimited,
        code @ 500..=599 => ModelError::ServerError(code),
        401 | 403 => ModelError::Auth(body),
        400 | 413 | 422 if looks_like_context_overflow(&body) => ModelError::ContextOverflow,
        400 | 404 | 413 | 422 => ModelError::InvalidRequest(body),
        _ => ModelError::BadResponse(format!("{} {}", status, body)),
    }
}

pub fn classify_reqwest(err: reqwest::Error) -> ModelError {
    if err.is_timeout() {
        ModelError::Timeout
    } else {
        ModelError::Network(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: u16) -> StatusCode {
        StatusCode::from_u16(code).unwrap()
    }

    #[test]
    fn classify_rate_limit_and_server_error() {
        assert!(matches!(
            classify_status(status(429), String::new()),
            ModelError::RateLimited
        ));
        assert!(matches!(
            classify_status(status(503), String::new()),
            ModelError::ServerError(503)
        ));
    }

    #[test]
    fn classify_auth_truncates_body() {
        let long = "x".repeat(1000);
        match classify_status(status(401), long) {
            ModelError::Auth(b) => assert!(b.len() <= 500),
            other => panic!("expected Auth, got {:?}", other),
        }
    }

    #[test]
    fn classify_context_overflow_variants() {
        let bodies = [
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#,
            r#"{"error":"prompt is too long: 90000 tokens > 8192"}}"#,
            r#"{"error":{"code":"context_length_exceeded","message":"maximum context length exceeded"}}"#,
            r#"inputs too many tokens"#,
        ];
        for b in bodies {
            assert!(
                matches!(
                    classify_status(status(400), b.to_string()),
                    ModelError::ContextOverflow
                ),
                "body: {}",
                b
            );
        }
    }

    #[test]
    fn classify_generic_bad_request_is_invalid_request() {
        assert!(matches!(
            classify_status(status(400), r#"{"error":"bad model id"}"#.into()),
            ModelError::InvalidRequest(_)
        ));
        assert!(matches!(
            classify_status(status(404), String::new()),
            ModelError::InvalidRequest(_)
        ));
    }

    #[test]
    fn fallback_semantics_of_retriable() {
        assert!(ModelError::RateLimited.is_retriable());
        assert!(ModelError::ServerError(500).is_retriable());
        assert!(ModelError::Timeout.is_retriable());
        assert!(ModelError::Network("x".into()).is_retriable());
        assert!(ModelError::ContextOverflow.is_retriable());
        assert!(ModelError::LocalUnavailable("refused".into()).is_retriable());
        assert!(ModelError::ProviderMissing("ollama".into()).is_retriable());

        assert!(!ModelError::Auth("bad".into()).is_retriable());
        assert!(!ModelError::InvalidRequest("x".into()).is_retriable());
        assert!(!ModelError::BadResponse("x".into()).is_retriable());
        assert!(!ModelError::Unsupported("x".into()).is_retriable());
    }

    #[test]
    fn auth_detection_flag() {
        assert!(ModelError::Auth("k".into()).is_auth());
        assert!(!ModelError::RateLimited.is_auth());
    }

    #[test]
    fn display_messages_are_chinese_and_read() {
        assert_eq!(
            ModelError::ContextOverflow.to_string(),
            "对话超出模型上下文窗口"
        );
        assert_eq!(
            ModelError::LocalUnavailable("connection refused".into()).to_string(),
            "本地模型服务不可用：connection refused"
        );
        let s = ModelError::RateLimited.to_string();
        assert!(s.contains("429"));
    }
}
