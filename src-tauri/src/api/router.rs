use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use serde_json::Value;

use super::client::Fallback;
use super::compat::CompatProvider;
use super::config::{ProviderConfig, RouterConfig, CONFIG_KEY};
use super::error::ModelError;
use super::provider::ProviderKind;
use super::sse::ChatStream;

/// 降级链路由中心（v0.2.0 设计，v0.2.1 多执行器）：
/// 持有可热更新的配置与各供应商执行器，统一提供带降级的流式/一次性请求。
///
/// 执行器按 `ProviderConfig` 构建（OpenAI 兼容族），链上条目按其
/// `provider` 字段路由到对应执行器；执行器缺失的条目跳过继续降级。
pub struct Router {
    config: RwLock<RouterConfig>,
    executors: Mutex<HashMap<String, CompatProvider>>,
}

/// 降级链上的一个候选条目（索引即会话 `model_idx` 语义）。
struct ChainEntry {
    idx: usize,
    provider: String,
    model_id: String,
    #[allow(dead_code)]
    label: String,
}

fn build_executor(p: &ProviderConfig) -> CompatProvider {
    let key = {
        let k = p.api_key.trim();
        if k.is_empty() {
            None
        } else {
            Some(k.to_string())
        }
    };
    match p.kind {
        ProviderKind::Azure => CompatProvider::with_azure(
            &p.id,
            &p.base_url,
            p.deployment.as_deref().unwrap_or_default(),
            p.api_version.as_deref().unwrap_or("2024-02-16"),
            key,
        ),
        _ => CompatProvider::with_models_path(
            &p.id,
            p.kind,
            &p.base_url,
            key,
            p.headers.clone(),
            p.models_path.clone(),
        ),
    }
}

fn build_executors(cfg: &RouterConfig) -> HashMap<String, CompatProvider> {
    cfg.providers
        .iter()
        .filter(|p| p.enabled)
        .map(|p| (p.id.clone(), build_executor(p)))
        .collect()
}

impl Router {
    pub fn new(config: RouterConfig) -> Self {
        let executors = build_executors(&config);
        Self {
            config: RwLock::new(config),
            executors: Mutex::new(executors),
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
                // 回环地址的兼容端点（vLLM/LM Studio）无需密钥
                _ => {
                    !p.api_key.trim().is_empty()
                        || super::compat::is_loopback_base(&p.base_url)
                }
            })
    }

    /// 更新指定供应商的 API Key（内存态，持久化由调用方负责）。
    pub fn set_provider_key(&self, id: &str, key: Option<String>) {
        let new_key = key.unwrap_or_default();
        let snapshot = {
            let Ok(mut cfg) = self.config.write() else {
                return;
            };
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == id) {
                p.api_key = new_key;
            }
            cfg.clone()
        };
        if let Ok(mut exec) = self.executors.lock() {
            *exec = build_executors(&snapshot);
        }
    }

    /// 整体替换配置并重建执行器（设置页保存提供商列表后调用）。
    pub fn set_config(&self, cfg: RouterConfig) {
        let executors = build_executors(&cfg);
        if let Ok(mut guard) = self.config.write() {
            *guard = cfg;
        }
        if let Ok(mut guard) = self.executors.lock() {
            *guard = executors;
        }
    }

    /// 取指定供应商执行器（共享 HTTP 连接池的克隆句柄）。
    pub fn executor_for(&self, id: &str) -> Option<CompatProvider> {
        self.executors.lock().ok()?.get(id).cloned()
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

    /// 判断指定索引的模型是否为本地模型（Ollama / llama.cpp 等）。
    pub fn is_local_model(&self, idx: usize) -> bool {
        let Ok(cfg) = self.config.read() else {
            return false;
        };
        if cfg.models.is_empty() {
            return false;
        }
        let m = match cfg.models.get(idx.min(cfg.models.len().saturating_sub(1))) {
            Some(m) => m,
            None => return false,
        };
        cfg.providers
            .iter()
            .find(|p| p.id == m.provider)
            .map(|p| p.kind.is_local())
            .unwrap_or(false)
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
                if !m.enabled || !m.pinned || !provider_enabled {
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
        F: Fn(usize, String, String) -> Fut,
        Fut: std::future::Future<Output = Result<T, ModelError>>,
    {
        let chain = self.chain(start_idx);
        if chain.is_empty() {
            return Err(ModelError::BadResponse(
                "没有可用的模型（降级链为空）".into(),
            ));
        }
        let start_clamped = start_idx.min(
            self.config
                .read()
                .map(|c| c.models.len().saturating_sub(1))
                .unwrap_or(start_idx),
        );
        let total = chain.len();
        let mut first_err: Option<ModelError> = None;
        for (pos, entry) in chain.iter().enumerate() {
            match f(entry.idx, entry.provider.clone(), entry.model_id.clone()).await {
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
        self.with_fallback(start_idx, move |_idx, provider, model_id| {
            let exec = self.executor_for(&provider);
            async move {
                match exec {
                    Some(exec) => exec.stream(&model_id, messages).await,
                    None => Err(ModelError::ProviderMissing(provider)),
                }
            }
        })
        .await
    }

    pub async fn complete_with_fallback(
        &self,
        start_idx: usize,
        messages: &[Value],
    ) -> Result<Fallback<String>, ModelError> {
        self.with_fallback(start_idx, move |_idx, provider, model_id| {
            let exec = self.executor_for(&provider);
            async move {
                match exec {
                    Some(exec) => exec.complete(&model_id, messages).await,
                    None => Err(ModelError::ProviderMissing(provider)),
                }
            }
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
    use crate::api::provider::{ChatProvider, ProviderKind};

    fn test_router() -> Router {
        Router::new(default_config())
    }

    fn ollama_provider() -> ProviderConfig {
        ProviderConfig {
            id: "ollama".into(),
            preset: None,
            kind: ProviderKind::Ollama,
            name: "Ollama".into(),
            base_url: "http://localhost:11434".into(),
            api_key: String::new(),
            headers: serde_json::Map::new(),
            models_path: None,
            deployment: None,
            api_version: None,
            enabled: true,
        }
    }

    #[tokio::test]
    async fn fallback_walks_enabled_chain_only() {
        let mut cfg = default_config();
        // 禁用第 2 个模型：链上应跳过它
        cfg.models[1].enabled = false;
        let router = Router::new(cfg);
        let err = router
            .with_fallback(0, |idx, _pid, _mid| async move {
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
            .with_fallback(99, |idx, _pid, _mid| async move { Ok(idx) })
            .await
            .unwrap();
        assert_eq!(out.model_idx, 3);
        assert!(out.notice.is_none());
    }

    #[tokio::test]
    async fn fallback_auth_error_stops_immediately() {
        let router = test_router();
        let err = router
            .with_fallback(0, |_idx, _pid, _mid| async {
                Err::<(), _>(ModelError::Auth("bad".into()))
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ModelError::Auth(_)));
    }

    #[tokio::test]
    async fn fallback_covers_multi_provider_chain() {
        let mut cfg = default_config();
        cfg.models.push(ModelConfig {
            provider: "ollama".into(),
            id: "qwen3:0.6b".into(),
            label: "Qwen3-0.6b".into(),
            context_limit: 32768,
            enabled: true,
            pinned: true,
            tags: vec![],
        });
        cfg.providers.push(ollama_provider());
        let router = Router::new(cfg);
        // 起点即 ollama 条目：多执行器下它同样在降级链中
        let out = router
            .with_fallback(4, |idx, _pid, _mid| async move { Ok(idx) })
            .await
            .unwrap();
        assert_eq!(out.model_idx, 4);
        assert!(out.notice.is_none());
    }

    #[tokio::test]
    async fn missing_executor_skips_to_next_provider() {
        let mut cfg = default_config();
        // openrouter 与 ollama 各一模型，openrouter provider 被禁用
        // → 执行器不存在，链上 openrouter 条目报 ProviderMissing 并继续
        cfg.models.push(ModelConfig {
            provider: "ollama".into(),
            id: "qwen3:0.6b".into(),
            label: "Qwen3-0.6b".into(),
            context_limit: 32768,
            enabled: true,
            pinned: true,
            tags: vec![],
        });
        cfg.providers.push(ollama_provider());
        cfg.providers[0].enabled = false;
        let router = Router::new(cfg);
        let err = router
            .with_fallback(0, |idx, pid, _mid| async move {
                if pid == "openrouter" {
                    Err(ModelError::ProviderMissing(pid))
                } else {
                    Ok(idx)
                }
            })
            .await
            .unwrap();
        // openrouter 条目缺失执行器 → 跳过 → ollama 条目成功
        assert_eq!(err.model_idx, 4);
        assert!(err.notice.is_some());
    }

    #[test]
    fn executors_built_from_enabled_providers() {
        let mut cfg = default_config();
        cfg.providers.push(ollama_provider());
        let router = Router::new(cfg);
        assert!(router.executor_for("openrouter").is_some());
        let ollama = router.executor_for("ollama").expect("ollama 执行器");
        assert_eq!(ollama.kind(), ProviderKind::Ollama);
        assert!(router.executor_for("ghost").is_none());

        // 禁用后重建即消失
        let mut cfg = router.config();
        cfg.providers[1].enabled = false;
        router.set_config(cfg);
        assert!(router.executor_for("ollama").is_none());
        assert!(router.executor_for("openrouter").is_some());
    }

    #[test]
    fn credentials_and_key_hot_reload() {
        let router = test_router();
        assert!(!router.has_credentials());

        router.set_provider_key("openrouter", Some("sk-test".into()));
        assert!(router.has_credentials());
        assert_eq!(router.config().providers[0].api_key, "sk-test");
        assert!(router.executor_for("openrouter").unwrap().has_api_key());

        router.set_provider_key("openrouter", None);
        assert!(!router.has_credentials());
        assert!(!router.executor_for("openrouter").unwrap().has_api_key());

        // 本地供应商无需 key
        let mut cfg = default_config();
        cfg.providers.push(ollama_provider());
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

    #[test]
    fn mixed_fallback_chain_local_then_cloud() {
        // 本地 + 云端混合降级链：本地模型在前，云端在后
        let mut cfg = default_config();
        // 将本地模型插入到最前面
        cfg.models.insert(0, ModelConfig {
            provider: "ollama".into(),
            id: "qwen3:0.6b".into(),
            label: "Qwen3 0.6B".into(),
            context_limit: 8192,
            enabled: true,
            pinned: true,
            tags: vec!["local".into()],
        });
        cfg.providers.push(ProviderConfig {
            id: "ollama".into(),
            preset: None,
            kind: ProviderKind::Ollama,
            name: "Ollama".into(),
            base_url: "http://127.0.0.1:11434".into(),
            api_key: String::new(),
            headers: serde_json::Map::new(),
            models_path: None,
            deployment: None,
            api_version: None,
            enabled: true,
        });
        // 选中本地模型（现在在索引 0）
        cfg.selected = model_key("ollama", "qwen3:0.6b");

        let router = Router::new(cfg);
        let chain = router.chain(0);
        // 链应包含本地模型，然后是所有启用的云端模型
        assert!(!chain.is_empty());
        assert_eq!(chain[0].provider, "ollama");
        // 云端模型也在链中
        assert!(chain.iter().any(|e| e.provider == "openrouter"));
    }

    #[test]
    fn chain_includes_badcloud_without_key() {
        // 模拟某供应商执行器缺失（如配置错误），应跳过并继续下一个
        let mut cfg = default_config();
        // 添加一个启用但执行器构建失败的供应商（API key 为空的云端）
        cfg.providers.push(ProviderConfig {
            id: "badcloud".into(),
            preset: None,
            kind: ProviderKind::OpenAi,
            name: "Bad Cloud".into(),
            base_url: "https://api.example.com".into(),
            api_key: String::new(),
            headers: serde_json::Map::new(),
            models_path: None,
            deployment: None,
            api_version: None,
            enabled: true,
        });
        cfg.models.push(ModelConfig {
            provider: "badcloud".into(),
            id: "bad-model".into(),
            label: "Bad Model".into(),
            context_limit: 4096,
            enabled: true,
            pinned: true,
            tags: vec![],
        });
        cfg.selected = model_key("badcloud", "bad-model");

        let router = Router::new(cfg);
        // badcloud 无 key，执行器仍会创建但 ready=false
        let chain = router.chain(0);
        // 链应包含所有启用模型
        assert!(chain.iter().any(|e| e.provider == "badcloud"));
        // openrouter 在后
        assert!(chain.iter().any(|e| e.provider == "openrouter"));
    }
}
