use std::sync::RwLock;

use serde_json::Value;

use super::client::{Fallback, OpenRouterProvider};
use super::config::{RouterConfig, CONFIG_KEY};
use super::error::ModelError;
use super::provider::ProviderKind;
use super::sse::ChatStream;

/// 降级链路由中心（v0.2.0）：持有可热更新的配置与各 Provider 执行器，
/// 统一提供带降级的流式/一次性请求。
///
/// v0.2.0 仅接入 OpenRouter 执行器；链上其他供应商条目暂跳过，
/// v0.2.1+ 按配置构建多执行器后自然生效。
pub struct Router {
    config: RwLock<RouterConfig>,
    openrouter: OpenRouterProvider,
}

/// 降级链上的一个候选条目（索引即会话 `model_idx` 语义）。
struct ChainEntry {
    idx: usize,
    provider: String,
    model_id: String,
    label: String,
}

impl Router {
    pub fn new(config: RouterConfig) -> Self {
        let openrouter = OpenRouterProvider::openrouter();
        let key = config
            .providers
            .iter()
            .find(|p| p.id == "openrouter")
            .map(|p| p.api_key.trim().to_string())
            .filter(|k| !k.is_empty());
        openrouter.set_api_key(key);
        Self {
            config: RwLock::new(config),
            openrouter,
        }
    }

    pub fn config(&self) -> RouterConfig {
        self.config
            .read()
            .map(|c| c.clone())
            .unwrap_or_else(|_| super::config::default_config())
    }

    /// 是否至少有一个启用的供应商具备发起请求的条件。
    pub fn has_credentials(&self) -> bool {
        let Ok(cfg) = self.config.read() else {
            return false;
        };
        cfg.providers
            .iter()
            .filter(|p| p.enabled)
            .any(|p| match p.kind {
                ProviderKind::Ollama | ProviderKind::LlamaCpp => true,
                _ => !p.api_key.trim().is_empty(),
            })
    }

    /// 更新指定供应商的 API Key（仅内存态，持久化由调用方负责）。
    pub fn set_provider_key(&self, id: &str, key: Option<String>) {
        if let Ok(mut cfg) = self.config.write() {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == id) {
                p.api_key = key.unwrap_or_default();
            }
            let openrouter_key = cfg
                .providers
                .iter()
                .find(|p| p.id == "openrouter")
                .map(|p| p.api_key.trim().to_string())
                .filter(|k| !k.is_empty());
            drop(cfg);
            self.openrouter.set_api_key(openrouter_key);
        }
    }

    /// 整体替换配置（设置页保存提供商/模型列表后调用）。
    pub fn set_config(&self, cfg: RouterConfig) {
        let openrouter_key = cfg
            .providers
            .iter()
            .find(|p| p.id == "openrouter")
            .map(|p| p.api_key.trim().to_string())
            .filter(|k| !k.is_empty());
        self.openrouter.set_api_key(openrouter_key);
        if let Ok(mut guard) = self.config.write() {
            *guard = cfg;
        }
    }

    /// 当前选中模型在 `config.models` 中的索引（会话 model_idx 语义）。
    pub fn selected_index(&self) -> usize {
        let Ok(cfg) = self.config.read() else {
            return 0;
        };
        cfg.models
            .iter()
            .position(|m| m.key() == cfg.selected)
            .unwrap_or(0)
    }

    /// 指定索引模型的上下文窗口（越界收敛，缺省 64K）。
    pub fn context_limit(&self, idx: usize) -> usize {
        let Ok(cfg) = self.config.read() else {
            return 65536;
        };
        if cfg.models.is_empty() {
            return 65536;
        }
        cfg.models
            .get(idx.min(cfg.models.len() - 1))
            .map(|m| m.context_limit)
            .unwrap_or(65536)
    }

    /// 从 start_idx 起构建降级链：跳过被禁用的供应商与模型。
    fn chain(&self, start_idx: usize) -> Vec<ChainEntry> {
        let Ok(cfg) = self.config.read() else {
            return Vec::new();
        };
        if cfg.models.is_empty() {
            return Vec::new();
        }
        let start = start_idx.min(cfg.models.len() - 1);
        (start..cfg.models.len())
            .filter_map(|i| {
                let m = &cfg.models[i];
                let provider_enabled = cfg
                    .providers
                    .iter()
                    .find(|p| p.id == m.provider)
                    .map(|p| p.enabled)
                    .unwrap_or(false);
                if !m.enabled || !provider_enabled {
                    return None;
                }
                Some(ChainEntry {
                    idx: i,
                    provider: m.provider.clone(),
                    model_id: m.id.clone(),
                    label: m.label.clone(),
                })
            })
            .collect()
    }

    async fn with_fallback<T, F, Fut>(&self, start_idx: usize, f: F) -> Result<Fallback<T>, ModelError>
    where
        F: Fn(usize, String) -> Fut,
        Fut: std::future::Future<Output = Result<T, ModelError>>,
    {
        let chain = self.chain(start_idx);
        if chain.is_empty() {
            return Err(ModelError::BadResponse(
                "没有可用的模型（降级链为空）".into(),
            ));
        }
        // v0.2.0：仅 OpenRouter 执行器，其余供应商条目暂不可执行
        let runnable: Vec<&ChainEntry> = chain
            .iter()
            .filter(|e| e.provider == "openrouter")
            .collect();
        if runnable.is_empty() {
            return Err(ModelError::Unsupported(
                "当前版本仅接入 OpenRouter，请在设置中检查模型供应商".into(),
            ));
        }
        let start_clamped = start_idx.min(
            self.config
                .read()
                .map(|c| c.models.len().saturating_sub(1))
                .unwrap_or(start_idx),
        );
        let total = runnable.len();
        let mut first_err: Option<ModelError> = None;
        for (pos, entry) in runnable.iter().enumerate() {
            match f(entry.idx, entry.model_id.clone()).await {
                Ok(value) => {
                    let notice = if entry.idx != start_clamped {
                        let reason = match &first_err {
                            Some(ModelError::RateLimited) => "限流",
                            _ => "故障",
                        };
                        Some(format!(
                            "主模型{}，已自动降级至 {}",
                            reason, entry.label
                        ))
                    } else {
                        None
                    };
                    return Ok(Fallback {
                        value,
                        model_idx: entry.idx,
                        notice,
                    });
                }
                Err(err) if err.is_retriable() && pos + 1 < total => {
                    first_err.get_or_insert(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(first_err.unwrap_or_else(|| {
            ModelError::BadResponse("model list empty".into())
        }))
    }

    pub async fn stream_with_fallback(
        &self,
        start_idx: usize,
        messages: &[Value],
    ) -> Result<Fallback<ChatStream>, ModelError> {
        self.with_fallback(start_idx, move |_idx, model_id| async move {
            self.openrouter.stream(&model_id, messages).await
        })
        .await
    }

    pub async fn complete_with_fallback(
        &self,
        start_idx: usize,
        messages: &[Value],
    ) -> Result<Fallback<String>, ModelError> {
        self.with_fallback(start_idx, move |_idx, model_id| async move {
            self.openrouter.complete(&model_id, messages).await
        })
        .await
    }

    /// 供持久化键名引用（与 config 模块共享同一键）。
    pub fn config_key() -> &'static str {
        CONFIG_KEY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::config::{default_config, model_key, ModelConfig, ProviderConfig};
    use crate::api::provider::ProviderKind;

    fn test_router() -> Router {
        Router::new(default_config())
    }

    #[tokio::test]
    async fn fallback_walks_enabled_chain_only() {
        let mut cfg = default_config();
        // 禁用第 2 个模型：链上应跳过它
        cfg.models[1].enabled = false;
        let router = Router::new(cfg);
        let err = router
            .with_fallback(0, |idx, _model_id| async move {
                if idx == 0 {
                    Err(ModelError::RateLimited)
                } else {
                    Ok(idx)
                }
            })
            .await
            .unwrap();
        // 0 失败 → 跳过 1（禁用）→ 2 成功
        assert_eq!(err.model_idx, 2);
        assert!(err.notice.is_some());
    }

    #[tokio::test]
    async fn fallback_starts_from_clamped_index() {
        let router = test_router();
        // 越界的 start_idx 必须收敛，不得 panic
        let out = router
            .with_fallback(99, |idx, _model_id| async move { Ok(idx) })
            .await
            .unwrap();
        assert_eq!(out.model_idx, 3);
        assert!(out.notice.is_none());
    }

    #[tokio::test]
    async fn fallback_auth_error_stops_immediately() {
        let router = test_router();
        let err = router
            .with_fallback(0, |_idx, _model_id| async {
                Err::<(), _>(ModelError::Auth("bad".into()))
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ModelError::Auth(_)));
    }

    #[tokio::test]
    async fn fallback_skips_foreign_provider_entries() {
        let mut cfg = default_config();
        cfg.models.push(ModelConfig {
            provider: "ollama".into(),
            id: "qwen3:0.6b".into(),
            label: "Qwen3-0.6b".into(),
            context_limit: 32768,
            enabled: true,
            tags: vec![],
        });
        cfg.providers.push(ProviderConfig {
            id: "ollama".into(),
            kind: ProviderKind::Ollama,
            name: "Ollama".into(),
            base_url: "http://localhost:11434".into(),
            api_key: String::new(),
            headers: serde_json::Map::new(),
            enabled: true,
        });
        // 起点即 ollama 条目：v0.2.0 无本地执行器 → 该条目被跳过，
        // 但链上没有 openrouter 条目（起点在末尾）→ Unsupported
        let router = Router::new(cfg);
        let err = router
            .with_fallback(4, |_idx, _model_id| async { Ok(()) })
            .await
            .unwrap_err();
        assert!(matches!(err, ModelError::Unsupported(_)));
    }

    #[test]
    fn credentials_and_key_hot_reload() {
        let router = test_router();
        assert!(!router.has_credentials());

        router.set_provider_key("openrouter", Some("sk-test".into()));
        assert!(router.has_credentials());
        assert_eq!(
            router.config().providers[0].api_key,
            "sk-test"
        );

        router.set_provider_key("openrouter", None);
        assert!(!router.has_credentials());

        // 本地供应商无需 key
        let mut cfg = default_config();
        cfg.providers.push(ProviderConfig {
            id: "ollama".into(),
            kind: ProviderKind::Ollama,
            name: "Ollama".into(),
            base_url: "http://localhost:11434".into(),
            api_key: String::new(),
            headers: serde_json::Map::new(),
            enabled: true,
        });
        let router = Router::new(cfg);
        assert!(router.has_credentials());
    }

    #[test]
    fn selected_index_follows_config() {
        let mut cfg = default_config();
        let target = model_key("openrouter", &cfg.models[2].id);
        cfg.selected = target;
        let router = Router::new(cfg);
        assert_eq!(router.selected_index(), 2);
    }

    #[test]
    fn chain_skips_disabled_provider() {
        let mut cfg = default_config();
        cfg.providers[0].enabled = false;
        let router = Router::new(cfg);
        assert!(router.chain(0).is_empty());
    }
}
