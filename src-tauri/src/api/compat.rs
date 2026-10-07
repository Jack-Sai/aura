use std::sync::Arc;
use std::sync::RwLock;

use serde_json::Value;

use super::error::{classify_reqwest, classify_status, ModelError};
use super::provider::{ChatProvider, ProbeStatus, ProviderKind, RemoteModel};
use super::sse::ChatStream;

/// 端点拼装规则（OpenAI 兼容族通用）：
/// 1. base 已含 `/chat/completions` → 原样使用
/// 2. base 以 `/v1` 结尾 → 追加 `/chat/completions`
/// 3. 否则（如 `https://api.openai.com`、`http://localhost:11434`）→ 追加 `/v1/chat/completions`
/// Azure 走 deployment URL 拼装，不适用以上规则。
pub fn chat_endpoint(kind: ProviderKind, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if kind == ProviderKind::Azure {
        // Azure 需要 deployment 与 api_version，调用方应使用 azure_chat_endpoint
        return String::new();
    }
    if base.contains("/chat/completions") {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else {
        format!("{}/v1/chat/completions", base)
    }
}

/// Azure OpenAI：`{base}/openai/deployments/{deployment}/chat/completions?api-version={ver}`
pub fn azure_chat_endpoint(base_url: &str, deployment: &str, api_version: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!(
        "{}/openai/deployments/{}/chat/completions?api-version={}",
        base, deployment, api_version
    )
}

/// base 是否指向本机回环地址（localhost/127.0.0.1/::1）。
/// 回环上的 OpenAI 兼容端点（vLLM/LM Studio 等本地框架）通常无需密钥。
pub fn is_loopback_base(url: &str) -> bool {
    let rest = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(v) => v.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// 模型列表端点（同 chat 规则，将尾部替换为 /models）。
pub fn models_endpoint(kind: ProviderKind, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.contains("/chat/completions") {
        return base
            .trim_end_matches("/chat/completions")
            .to_string()
            + "/models";
    }
    if kind == ProviderKind::Ollama {
        return format!("{}/api/tags", base);
    }
    if base.ends_with("/v1") {
        format!("{}/models", base)
    } else {
        format!("{}/v1/models", base)
    }
}

/// OpenAI 兼容通用执行器：覆盖 OpenRouter / OpenAI / Azure / Ollama /
/// llama.cpp / vLLM / LM Studio / 自定义端点，差异仅在于端点拼装与鉴权头。
#[derive(Clone)]
pub struct CompatProvider {
    id: String,
    kind: ProviderKind,
    base_url: String,
    /// Azure deployment 名（kind == Azure 时必填）
    deployment: Option<String>,
    /// Azure api-version（kind == Azure 时必填）
    api_version: Option<String>,
    api_key: Arc<RwLock<Option<String>>>,
    /// 附加请求头（自定义供应商）
    headers: serde_json::Map<String, Value>,
    http: reqwest::Client,
}

impl CompatProvider {
    pub fn new(
        id: &str,
        kind: ProviderKind,
        base_url: &str,
        api_key: Option<String>,
        headers: serde_json::Map<String, Value>,
    ) -> Self {
        let is_local = is_loopback_base(base_url);
        let mut builder = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10));
        // 本地服务启用连接池保活，减少 TCP 握手延迟
        if is_local {
            builder = builder
                .pool_idle_timeout(std::time::Duration::from_secs(120))
                .pool_max_idle_per_host(8);
        }
        let http = builder.build().expect("failed to build http client");
        Self {
            id: id.to_string(),
            kind,
            base_url: base_url.trim_end_matches('/').to_string(),
            deployment: None,
            api_version: None,
            api_key: Arc::new(RwLock::new(api_key.filter(|k| !k.trim().is_empty()))),
            headers,
            http,
        }
    }

    /// Azure 专用构造：补齐 deployment 与 api-version。
    pub fn with_azure(
        id: &str,
        base_url: &str,
        deployment: &str,
        api_version: &str,
        api_key: Option<String>,
    ) -> Self {
        let mut p = Self::new(
            id,
            ProviderKind::Azure,
            base_url,
            api_key,
            serde_json::Map::new(),
        );
        p.deployment = Some(deployment.to_string());
        p.api_version = Some(api_version.to_string());
        p
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.read().map(|k| k.is_some()).unwrap_or(false)
    }

    pub fn set_api_key(&self, key: Option<String>) {
        if let Ok(mut g) = self.api_key.write() {
            *g = key;
        }
    }

    pub fn endpoint(&self) -> String {
        match self.kind {
            ProviderKind::Azure => azure_chat_endpoint(
                &self.base_url,
                self.deployment.as_deref().unwrap_or(""),
                self.api_version.as_deref().unwrap_or("2024-02-16"),
            ),
            _ => chat_endpoint(self.kind, &self.base_url),
        }
    }

    pub fn models_url(&self) -> String {
        models_endpoint(self.kind, &self.base_url)
    }

    fn request(&self, model: &str, messages: &[Value], stream: bool) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .post(self.endpoint())
            .json(&serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": stream,
                "max_tokens": 16384,
            }));
        if self.kind == ProviderKind::OpenRouter {
            req = req.header("X-Title", "Aura");
        }
        let key = self.api_key.read().ok().and_then(|g| g.clone());
        match self.kind {
            // Azure 用 api-key 请求头
            ProviderKind::Azure => {
                if let Some(key) = key {
                    req = req.header("api-key", key);
                }
            }
            // 本地服务（Ollama/llama.cpp）无需鉴权
            ProviderKind::Ollama | ProviderKind::LlamaCpp => {}
            _ => {
                if let Some(key) = key {
                    req = req.bearer_auth(key);
                }
            }
        }
        for (k, v) in &self.headers {
            if let Some(val) = v.as_str() {
                req = req.header(k, val);
            }
        }
        req
    }

    pub async fn stream(&self, model: &str, messages: &[Value]) -> Result<ChatStream, ModelError> {
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

    pub async fn complete(&self, model: &str, messages: &[Value]) -> Result<String, ModelError> {
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
            .map_err(|e| ModelError::BadResponse(e.to_string()))?;
        body["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ModelError::BadResponse("missing choices[0].message.content".into()))
    }

    async fn fetch_models(&self) -> Result<Vec<RemoteModel>, ModelError> {
        if self.kind == ProviderKind::Ollama {
            // Ollama 原生 /api/tags：{ models: [{name, details: {parameter_size}}] }
            let resp = self
                .http
                .get(self.models_url())
                .send()
                .await
                .map_err(classify_reqwest)?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(classify_status(status, body));
            }
            let body: Value = resp.json().await.map_err(|e| ModelError::BadResponse(e.to_string()))?;
            let out = body["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            let name = m["name"].as_str()?.to_string();
                            Some(RemoteModel {
                                id: name.clone(),
                                label: name,
                                context_limit: m["details"]["context_length"]
                                    .as_u64()
                                    .unwrap_or(8192) as usize,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Ok(out);
        }

        let mut req = self.http.get(self.models_url());
        let key = self.api_key.read().ok().and_then(|g| g.clone());
        match self.kind {
            ProviderKind::Azure => {
                if let Some(key) = key {
                    req = req.header("api-key", key);
                }
            }
            ProviderKind::LlamaCpp => {}
            _ => {
                if let Some(key) = key {
                    req = req.bearer_auth(key);
                }
            }
        }
        let resp = req.send().await.map_err(classify_reqwest)?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, body));
        }
        let body: Value = resp.json().await.map_err(|e| ModelError::BadResponse(e.to_string()))?;
        let out = body["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let id = m["id"].as_str()?.to_string();
                        let ctx = m["context_length"]
                            .as_u64()
                            .or_else(|| m["context_length"].as_f64().map(|f| f as u64));
                        Some(RemoteModel {
                            id: id.clone(),
                            label: id,
                            context_limit: ctx.unwrap_or(8192) as usize,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(out)
    }

    async fn do_probe(&self) -> Result<ProbeStatus, ModelError> {
        let started = std::time::Instant::now();
        let resp = self
            .http
            .get(self.models_url())
            .send()
            .await
            .map_err(classify_reqwest);
        let latency_ms = started.elapsed().as_millis() as u64;
        match resp {
            Ok(r) if r.status().is_success() => Ok(ProbeStatus {
                ok: true,
                latency_ms,
                message: "连接正常".to_string(),
            }),
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                let err = classify_status(status, body);
                Ok(ProbeStatus {
                    ok: false,
                    latency_ms,
                    message: err.to_string(),
                })
            }
            Err(e) => {
                let local = matches!(e, ModelError::Network(_) | ModelError::Timeout);
                Ok(ProbeStatus {
                    ok: false,
                    latency_ms,
                    message: if local {
                        "服务未启动或地址不可达".to_string()
                    } else {
                        e.to_string()
                    },
                })
            }
        }
    }
}

impl ChatProvider for CompatProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn ready(&self) -> bool {
        if !self.kind.requires_api_key() {
            return true;
        }
        if self.has_api_key() {
            return true;
        }
        // 本地回环的兼容端点（vLLM/LM Studio 等）通常无需密钥
        is_loopback_base(&self.base_url)
    }

    fn stream<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [Value],
    ) -> impl std::future::Future<Output = Result<ChatStream, ModelError>> + Send + 'a {
        CompatProvider::stream(self, model, messages)
    }

    fn complete<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [Value],
    ) -> impl std::future::Future<Output = Result<String, ModelError>> + Send + 'a {
        CompatProvider::complete(self, model, messages)
    }

    fn list_models(&self) -> impl std::future::Future<Output = Result<Vec<RemoteModel>, ModelError>> + Send + '_ {
        self.fetch_models()
    }

    fn probe(&self) -> impl std::future::Future<Output = Result<ProbeStatus, ModelError>> + Send + '_ {
        self.do_probe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_endpoint_rules() {
        // 1. 已含 /chat/completions 原样
        assert_eq!(
            chat_endpoint(ProviderKind::OpenAi, "https://x.com/v1/chat/completions"),
            "https://x.com/v1/chat/completions"
        );
        // 2. /v1 结尾
        assert_eq!(
            chat_endpoint(ProviderKind::OpenRouter, "https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            chat_endpoint(ProviderKind::OpenAi, "https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
        // 3. 裸域名自动补 /v1
        assert_eq!(
            chat_endpoint(ProviderKind::OpenAi, "https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
        // Ollama 裸地址 → /v1/chat/completions（OpenAI 兼容口）
        assert_eq!(
            chat_endpoint(ProviderKind::Ollama, "http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            chat_endpoint(ProviderKind::LlamaCpp, "http://127.0.0.1:8080"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
    }

    #[test]
    fn azure_endpoint_assembly() {
        assert_eq!(
            azure_chat_endpoint(
                "https://myres.openai.azure.com/",
                "gpt-4o",
                "2024-06-01"
            ),
            "https://myres.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-06-01"
        );
    }

    #[test]
    fn models_endpoint_rules() {
        assert_eq!(
            models_endpoint(ProviderKind::OpenAi, "https://api.openai.com"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_endpoint(ProviderKind::OpenAi, "https://api.openai.com/v1"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_endpoint(ProviderKind::Ollama, "http://localhost:11434"),
            "http://localhost:11434/api/tags"
        );
        assert_eq!(
            models_endpoint(ProviderKind::LlamaCpp, "http://127.0.0.1:8080"),
            "http://127.0.0.1:8080/v1/models"
        );
    }

    #[test]
    fn local_framework_endpoint_matrix() {
        // vLLM（默认监听 8000，OpenAI 兼容口在 /v1）
        assert_eq!(
            chat_endpoint(ProviderKind::Custom, "http://localhost:8000"),
            "http://localhost:8000/v1/chat/completions"
        );
        assert_eq!(
            models_endpoint(ProviderKind::Custom, "http://localhost:8000"),
            "http://localhost:8000/v1/models"
        );
        // LM Studio（默认 1234，OpenAI 兼容口）
        assert_eq!(
            chat_endpoint(ProviderKind::OpenAi, "http://localhost:1234"),
            "http://localhost:1234/v1/chat/completions"
        );
        assert_eq!(
            models_endpoint(ProviderKind::OpenAi, "http://localhost:1234"),
            "http://localhost:1234/v1/models"
        );
        // llama.cpp server：显式 /v1 与尾斜杠容错
        assert_eq!(
            chat_endpoint(ProviderKind::LlamaCpp, "http://127.0.0.1:8080/v1"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
        assert_eq!(
            models_endpoint(ProviderKind::LlamaCpp, "http://127.0.0.1:8080/v1"),
            "http://127.0.0.1:8080/v1/models"
        );
        assert_eq!(
            chat_endpoint(ProviderKind::LlamaCpp, "http://127.0.0.1:8080/"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
        // 本地框架无需鉴权即可就绪
        assert!(CompatProvider::new(
            "vlm",
            ProviderKind::Custom,
            "http://localhost:8000",
            None,
            Default::default(),
        )
        .ready());
        assert!(CompatProvider::new(
            "lms",
            ProviderKind::OpenAi,
            "http://localhost:1234",
            None,
            Default::default(),
        )
        .ready());
    }

    #[test]
    fn loopback_detection_for_local_frameworks() {
        assert!(is_loopback_base("http://localhost:8000"));
        assert!(is_loopback_base("http://127.0.0.1:1234/v1"));
        assert!(is_loopback_base("http://LOCALHOST:8080"));
        assert!(is_loopback_base("http://[::1]:8080"));
        assert!(is_loopback_base("https://127.0.0.1"));
        assert!(!is_loopback_base("https://api.openai.com"));
        assert!(!is_loopback_base("http://192.168.1.5:8000"));
        assert!(!is_loopback_base("http://127.0.0.2"));
        assert!(!is_loopback_base(""));
    }

    #[test]
    fn auth_readiness_by_kind() {
        // 云端需要 key
        let p = CompatProvider::new("openai", ProviderKind::OpenAi, "https://api.openai.com", None, Default::default());
        assert!(!p.ready());
        p.set_api_key(Some("sk-x".into()));
        assert!(p.ready());

        // 本地无需 key
        let p = CompatProvider::new("ollama", ProviderKind::Ollama, "http://localhost:11434", None, Default::default());
        assert!(p.ready());

        // Azure 需要 key
        let p = CompatProvider::with_azure(
            "az",
            "https://myres.openai.azure.com",
            "dep",
            "2024-06-01",
            None,
        );
        assert!(!p.ready());
        assert_eq!(
            p.endpoint(),
            "https://myres.openai.azure.com/openai/deployments/dep/chat/completions?api-version=2024-06-01"
        );
    }

    #[test]
    fn openrouter_keeps_default_endpoint_and_title_header() {
        let p = CompatProvider::new(
            "openrouter",
            ProviderKind::OpenRouter,
            "https://openrouter.ai/api/v1",
            Some("sk-or".into()),
            Default::default(),
        );
        assert_eq!(
            p.endpoint(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert!(p.ready());
    }

    #[test]
    fn custom_headers_preserved() {
        let mut h = serde_json::Map::new();
        h.insert("X-Custom".into(), "v1".into());
        let p = CompatProvider::new("c", ProviderKind::Custom, "http://x:1", None, h);
        assert_eq!(p.id(), "c");
        assert_eq!(p.kind(), ProviderKind::Custom);
        assert_eq!(p.headers.get("X-Custom").and_then(|v| v.as_str()), Some("v1"));
    }

    #[tokio::test]
    async fn probe_unreachable_local_service_reports_offline() {
        // 未监听端口：快速失败，返回探测失败而非 panic
        let p = CompatProvider::new(
            "ollama",
            ProviderKind::Ollama,
            "http://127.0.0.1:59999",
            None,
            Default::default(),
        );
        let status = p.probe().await.unwrap();
        assert!(!status.ok);
        assert!(!status.message.is_empty());
    }

    #[tokio::test]
    async fn list_models_default_empty_via_trait() {
        let p = CompatProvider::new("c", ProviderKind::Custom, "http://127.0.0.1:59999", None, Default::default());
        let err = p.list_models().await.unwrap_err();
        // 连接失败被归类为网络错误
        assert!(matches!(err, ModelError::Network(_) | ModelError::Timeout));
    }
}
