use std::future::Future;

use serde_json::Value;

use super::error::ModelError;
use super::sse::ChatStream;

/// Provider 分类：决定 URL 拼装、鉴权方式、探测端点与提示策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
// 必须逐变体显式指定：默认 snake_case 会把 OpenRouter 写成 `open_router`，
// 与前端使用的 `openrouter` 不一致，配置将无法反序列化
pub enum ProviderKind {
    /// OpenRouter 云端
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// OpenAI 官方或任意 OpenAI 兼容端点
    #[serde(rename = "openai")]
    OpenAi,
    /// Azure OpenAI（api-key 头 + deployment URL）
    #[serde(rename = "azure")]
    Azure,
    /// Ollama 本地部署
    #[serde(rename = "ollama")]
    Ollama,
    /// llama.cpp server
    #[serde(rename = "llama_cpp")]
    LlamaCpp,
    /// 用户自定义 OpenAI 兼容端点
    #[serde(rename = "custom")]
    Custom,
}

impl ProviderKind {
    pub fn is_local(&self) -> bool {
        matches!(self, ProviderKind::Ollama | ProviderKind::LlamaCpp)
    }

    pub fn requires_api_key(&self) -> bool {
        !self.is_local() && !matches!(self, ProviderKind::LlamaCpp)
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProviderKind::OpenRouter => "OpenRouter",
            ProviderKind::OpenAi => "OpenAI",
            ProviderKind::Azure => "Azure OpenAI",
            ProviderKind::Ollama => "Ollama",
            ProviderKind::LlamaCpp => "llama.cpp",
            ProviderKind::Custom => "自定义",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "openrouter" => ProviderKind::OpenRouter,
            "openai" => ProviderKind::OpenAi,
            "azure" => ProviderKind::Azure,
            "ollama" => ProviderKind::Ollama,
            "llamacpp" => ProviderKind::LlamaCpp,
            _ => ProviderKind::Custom,
        }
    }
}

/// 从 Provider 拉取到的模型描述。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RemoteModel {
    pub id: String,
    pub label: String,
    pub context_limit: usize,
}

/// 连通性探测结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbeStatus {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
}

/// 统一模型供应商接口（v0.2.0 抽象层）。
///
/// 约束：返回 `impl Future + Send`（RPITIT），因此本 trait 不是对象安全的；
/// 调度方（Router）通过 enum dispatch 持有具体实现。
/// 实现者需保证：`ready()==false` 时 `stream/complete` 应返回 `Auth` 错误而非发请求。
pub trait ChatProvider: Send + Sync {
    /// 配置中的供应商 id（不含冒号，作为模型 key 前缀）
    fn id(&self) -> &str;

    fn kind(&self) -> ProviderKind;

    /// 是否已具备发起请求的凭据（API Key 等；本地 Provider 恒为 true）
    fn ready(&self) -> bool;

    fn stream<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [Value],
    ) -> impl Future<Output = Result<ChatStream, ModelError>> + Send + 'a;

    fn complete<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [Value],
    ) -> impl Future<Output = Result<String, ModelError>> + Send + 'a;

    /// 拉取可用模型列表；默认空实现（模型由配置手动维护的 Provider）。
    fn list_models(&self) -> impl Future<Output = Result<Vec<RemoteModel>, ModelError>> + Send + '_ {
        async { Ok(Vec::new()) }
    }

    /// 连通性探测；默认基于 `ready()` 的零成本实现。
    fn probe(&self) -> impl Future<Output = Result<ProbeStatus, ModelError>> + Send + '_ {
        async {
            Ok(ProbeStatus {
                ok: self.ready(),
                latency_ms: 0,
                message: if self.ready() {
                    "已配置 API Key".to_string()
                } else {
                    "未配置 API Key".to_string()
                },
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::CompatProvider;

    fn test_provider() -> CompatProvider {
        CompatProvider::new(
            "openrouter",
            ProviderKind::OpenRouter,
            "https://openrouter.ai/api/v1",
            None,
            Default::default(),
        )
    }

    fn assert_provider<P: ChatProvider>(p: &P) -> (String, ProviderKind, bool) {
        (p.id().to_string(), p.kind(), p.ready())
    }

    #[test]
    fn compat_provider_implements_trait() {
        let p = test_provider();
        let (id, kind, ready) = assert_provider(&p);
        assert_eq!(id, "openrouter");
        assert_eq!(kind, ProviderKind::OpenRouter);
        assert!(!ready);

        p.set_api_key(Some("sk-test".into()));
        assert!(p.ready());
    }

    #[tokio::test]
    async fn default_list_models_and_probe_report_key_state() {
        // 最小实现：list_models/probe 走 trait 默认（不发网络请求）
        struct Minimal {
            key: bool,
        }
        impl ChatProvider for Minimal {
            fn id(&self) -> &str {
                "minimal"
            }
            fn kind(&self) -> ProviderKind {
                ProviderKind::Custom
            }
            fn ready(&self) -> bool {
                self.key
            }
            fn stream<'a>(
                &'a self,
                _model: &'a str,
                _messages: &'a [Value],
            ) -> impl std::future::Future<Output = Result<ChatStream, ModelError>> + Send + 'a
            {
                async { unimplemented!("not used") }
            }
            fn complete<'a>(
                &'a self,
                _model: &'a str,
                _messages: &'a [Value],
            ) -> impl std::future::Future<Output = Result<String, ModelError>> + Send + 'a
            {
                async { unimplemented!("not used") }
            }
        }

        let p = Minimal { key: false };
        assert!(p.list_models().await.unwrap().is_empty());
        let status = p.probe().await.unwrap();
        assert!(!status.ok);
        assert!(status.message.contains("未配置"));

        let p = Minimal { key: true };
        let status = p.probe().await.unwrap();
        assert!(status.ok);
        assert!(status.message.contains("已配置"));
    }

    #[test]
    fn kind_classification() {
        assert!(ProviderKind::Ollama.is_local());
        assert!(ProviderKind::LlamaCpp.is_local());
        assert!(!ProviderKind::OpenAi.is_local());
        assert!(!ProviderKind::Ollama.requires_api_key());
        assert!(ProviderKind::OpenAi.requires_api_key());
        assert!(ProviderKind::OpenRouter.requires_api_key());
        assert_eq!(ProviderKind::from_id("ollama"), ProviderKind::Ollama);
        assert_eq!(ProviderKind::from_id("unknown"), ProviderKind::Custom);
        assert_eq!(ProviderKind::Azure.label(), "Azure OpenAI");
    }

    #[test]
    fn provider_kind_serde_roundtrip() {
        let k = ProviderKind::LlamaCpp;
        let json = serde_json::to_string(&k).unwrap();
        assert_eq!(json, "\"llama_cpp\"");
        let back: ProviderKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, k);

        // 与前端 ProviderKind 联合类型的取值逐一对应
        for (raw, expected) in [
            ("\"openrouter\"", ProviderKind::OpenRouter),
            ("\"openai\"", ProviderKind::OpenAi),
            ("\"azure\"", ProviderKind::Azure),
            ("\"ollama\"", ProviderKind::Ollama),
            ("\"llama_cpp\"", ProviderKind::LlamaCpp),
            ("\"custom\"", ProviderKind::Custom),
        ] {
            let back: ProviderKind = serde_json::from_str(raw).unwrap();
            assert_eq!(back, expected, "反序列化 {}", raw);
            assert_eq!(serde_json::to_string(&back).unwrap(), raw);
        }
        // 曾经的回归点：默认 snake_case 会产出 open_router 导致配置无法回读
        assert_ne!(
            serde_json::to_string(&ProviderKind::OpenRouter).unwrap(),
            "\"open_router\""
        );
    }
}
