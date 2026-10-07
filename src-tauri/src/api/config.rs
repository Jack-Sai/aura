use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::client::MODELS;
use super::provider::ProviderKind;

/// 持久化于 settings 表的配置键。
pub const CONFIG_KEY: &str = "model_config";

/// 模型供应商配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderConfig {
    /// 主键，不含冒号（作为模型 key 前缀）
    pub id: String,
    pub kind: ProviderKind,
    /// 展示名称
    pub name: String,
    /// OpenAI 兼容根地址（不含 /chat/completions 后缀）
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    /// 附加请求头（v0.2.6 自定义供应商）
    #[serde(default)]
    pub headers: serde_json::Map<String, serde_json::Value>,
    /// Azure deployment 名（kind == azure 时使用）
    #[serde(default)]
    pub deployment: Option<String>,
    /// Azure api-version（kind == azure 时使用）
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 单个可选模型；`models` 数组顺序即降级链顺序。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelConfig {
    pub provider: String,
    pub id: String,
    pub label: String,
    pub context_limit: usize,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 模型标签（如 small/local 策略标记）
    #[serde(default)]
    pub tags: Vec<String>,
}

impl ModelConfig {
    /// `{provider}:{model}` 复合 key（provider 不含冒号）。
    pub fn key(&self) -> String {
        model_key(&self.provider, &self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouterConfig {
    pub providers: Vec<ProviderConfig>,
    pub models: Vec<ModelConfig>,
    /// 当前选中模型 key（`{provider}:{id}`）
    #[serde(default)]
    pub selected: String,
}

pub fn model_key(provider: &str, model_id: &str) -> String {
    format!("{}:{}", provider, model_id)
}

/// 解析 `{provider}:{model}`；provider 段不含冒号，故首冒号切分。
pub fn parse_model_key(key: &str) -> Option<(&str, &str)> {
    let idx = key.find(':')?;
    let (provider, model) = key.split_at(idx);
    if provider.is_empty() || model.len() == 1 {
        return None;
    }
    Some((provider, &model[1..]))
}

/// 默认配置：OpenRouter + 与 v0.1.x 一致的四个模型（老用户无感升级）。
pub fn default_config() -> RouterConfig {
    RouterConfig {
        providers: vec![ProviderConfig {
            id: "openrouter".to_string(),
            kind: ProviderKind::OpenRouter,
            name: "OpenRouter".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            api_key: String::new(),
            headers: serde_json::Map::new(),
            deployment: None,
            api_version: None,
            enabled: true,
        }],
        models: MODELS
            .iter()
            .map(|m| ModelConfig {
                provider: "openrouter".to_string(),
                id: m.id.to_string(),
                label: m.label.to_string(),
                context_limit: m.context_limit,
                enabled: true,
                tags: Vec::new(),
            })
            .collect(),
        selected: model_key("openrouter", MODELS[0].id),
    }
}

/// 从 v0.1.x 遗留设置构造配置：合并 `openrouter_api_key` 与 `model_id`。
pub fn legacy_config(api_key: Option<String>, model_id: Option<String>) -> RouterConfig {
    let mut cfg = default_config();
    if let Some(key) = api_key {
        if !key.trim().is_empty() {
            cfg.providers[0].api_key = key;
        }
    }
    if let Some(mid) = model_id {
        let trimmed = mid.trim();
        if let Some(found) = cfg
            .models
            .iter()
            .position(|m| m.id == trimmed || m.key() == trimmed)
        {
            cfg.selected = cfg.models[found].key();
        }
    }
    cfg
}

/// 读取配置：`model_config` 不存在或损坏时回退遗留设置迁移。
pub fn load_config(conn: &Connection) -> RouterConfig {
    match crate::db::get_setting(conn, CONFIG_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<RouterConfig>(&json).ok())
    {
        Some(cfg) => normalize(cfg),
        None => legacy_config(
            crate::db::get_setting(conn, "openrouter_api_key")
                .ok()
                .flatten(),
            crate::db::get_setting(conn, "model_id").ok().flatten(),
        ),
    }
}

pub fn save_config(conn: &Connection, cfg: &RouterConfig) -> Result<(), String> {
    let json = serde_json::to_string(cfg).map_err(|e| e.to_string())?;
    crate::db::set_setting(conn, CONFIG_KEY, &json).map_err(|e| e.to_string())
}

/// 配置不变量：过滤供应商已不存在的模型；selected 必须指向
/// 当前可选（provider 与模型均启用）的条目，否则回退首个可选项。
pub fn normalize(mut cfg: RouterConfig) -> RouterConfig {
    cfg.models
        .retain(|m| cfg.providers.iter().any(|p| p.id == m.provider));
    let selectable: Vec<String> = cfg
        .models
        .iter()
        .filter(|m| {
            m.enabled
                && cfg
                    .providers
                    .iter()
                    .any(|p| p.id == m.provider && p.enabled)
        })
        .map(|m| m.key())
        .collect();
    if !selectable.contains(&cfg.selected) {
        cfg.selected = selectable
            .first()
            .cloned()
            .unwrap_or_else(|| model_key("openrouter", MODELS[0].id));
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> Connection {
        let path = std::env::temp_dir().join(format!(
            "aura_cfg_test_{}_{}.db",
            std::process::id(),
            rand_suffix()
        ));
        crate::db::open(&path).unwrap()
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as u64
    }

    #[test]
    fn model_key_roundtrip_with_openrouter_ids() {
        let id = "nvidia/nemotron-3.5-lightning:free";
        let key = model_key("openrouter", id);
        assert_eq!(key, "openrouter:nvidia/nemotron-3.5-lightning:free");
        let (p, m) = parse_model_key(&key).unwrap();
        assert_eq!(p, "openrouter");
        assert_eq!(m, id);

        assert!(parse_model_key("nocolon").is_none());
        assert!(parse_model_key(":empty").is_none());
        assert!(parse_model_key("p:").is_none());
    }

    #[test]
    fn default_config_matches_legacy_models() {
        let cfg = default_config();
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.models.len(), MODELS.len());
        assert_eq!(cfg.providers[0].id, "openrouter");
        assert_eq!(cfg.models[0].id, MODELS[0].id);
        assert_eq!(cfg.selected, cfg.models[0].key());
        assert!(cfg.models.iter().all(|m| m.provider == "openrouter"));
    }

    #[test]
    fn legacy_migration_merges_api_key_and_model_id() {
        let cfg = legacy_config(
            Some(" sk-or-v1-abc ".to_string()),
            Some(MODELS[2].id.to_string()),
        );
        assert_eq!(cfg.providers[0].api_key, " sk-or-v1-abc ");
        assert_eq!(cfg.selected, cfg.models[2].key());

        // 旧 model_id 已是复合 key 的情形
        let cfg = legacy_config(None, Some(model_key("openrouter", MODELS[1].id)));
        assert_eq!(cfg.selected, cfg.models[1].key());

        // 未知 model_id 不生效
        let cfg = legacy_config(None, Some("unknown/model".to_string()));
        assert_eq!(cfg.selected, cfg.models[0].key());
    }

    #[test]
    fn serde_roundtrip_preserves_structure() {
        let cfg = default_config();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: RouterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
        assert!(json.contains("openrouter"));
        assert!(json.contains("\"llama_cpp\"") || !json.contains("llama"));
    }

    #[test]
    fn normalize_drops_orphan_models_and_fixes_selected() {
        let mut cfg = default_config();
        cfg.models.push(ModelConfig {
            provider: "ghost".to_string(),
            id: "m1".to_string(),
            label: "Ghost".to_string(),
            context_limit: 8192,
            enabled: true,
            tags: vec![],
        });
        cfg.selected = model_key("ghost", "m1");
        let cfg = normalize(cfg);
        assert_eq!(cfg.models.len(), 4);
        assert_eq!(cfg.selected, cfg.models[0].key());

        // 禁用 provider 后其模型保留（可再启用），selected 若已不可选则回退
        let mut cfg = default_config();
        cfg.selected = cfg.models[2].key();
        cfg.providers[0].enabled = false;
        let cfg = normalize(cfg);
        assert_eq!(cfg.models.len(), 4);
        // 没有可选项时回退到出厂默认 key
        assert_eq!(cfg.selected, model_key("openrouter", MODELS[0].id));
    }

    #[test]
    fn load_config_migrates_legacy_settings() {
        let conn = temp_db();
        crate::db::set_setting(&conn, "openrouter_api_key", "sk-legacy").unwrap();
        crate::db::set_setting(&conn, "model_id", MODELS[1].id).unwrap();
        let cfg = load_config(&conn);
        assert_eq!(cfg.providers[0].api_key, "sk-legacy");
        assert_eq!(cfg.selected, cfg.models[1].key());

        // 持久化后再次加载走新配置
        save_config(&conn, &cfg).unwrap();
        let mut changed = cfg.clone();
        changed.selected = cfg.models[3].key();
        save_config(&conn, &changed).unwrap();
        let reloaded = load_config(&conn);
        assert_eq!(reloaded.selected, cfg.models[3].key());
    }

    #[test]
    fn load_config_falls_back_when_json_corrupt() {
        let conn = temp_db();
        crate::db::set_setting(&conn, CONFIG_KEY, "{not json").unwrap();
        crate::db::set_setting(&conn, "openrouter_api_key", "sk-x").unwrap();
        let cfg = load_config(&conn);
        assert_eq!(cfg.providers[0].api_key, "sk-x");
        assert_eq!(cfg.models.len(), MODELS.len());
    }
}
