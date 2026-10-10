use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::client::MODELS;
use super::provider::{ProviderKind, RemoteModel};

/// 持久化于 settings 表的配置键。
pub const CONFIG_KEY: &str = "model_config";

/// 模型供应商配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderConfig {
    /// 主键，不含冒号（作为模型 key 前缀）
    pub id: String,
    /// 引导用户选择的厂商预设 id（OpenRouter / DeepSeek / …）。
    /// 多个厂商共用同一 `kind`（均为 OpenAI 兼容），故需单独记录预设，
    /// 用于设置页回显厂商与模型列表分组标题。
    #[serde(default)]
    pub preset: Option<String>,
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
    /// 模型列表端点路径覆盖（相对 base_url）。
    /// DeepSeek 的列表端点是 `/models`（不带 `/v1`）；阿里百炼 chat 与
    /// models 端点不同构，均无法由 base 推导，故显式配置。
    #[serde(default)]
    pub models_path: Option<String>,
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
///
/// 远端拉取的模型默认 `pinned = false`（仅作为可选目录），用户勾选「收藏」
/// 后才置为 true 并进入降级链与对话页模型列表。
/// 旧配置无此字段时 serde 默认 true，即历史用户的所有模型都是已收藏。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelConfig {
    pub provider: String,
    pub id: String,
    pub label: String,
    pub context_limit: usize,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 是否已收藏（用户主动勾选）。
    #[serde(default = "default_true")]
    pub pinned: bool,
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
            preset: Some("openrouter".into()),
            kind: ProviderKind::OpenRouter,
            name: "OpenRouter".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            api_key: String::new(),
            headers: serde_json::Map::new(),
            models_path: None,
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
                pinned: true,
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
/// 把出厂默认模型的旧简称刷成与 id 主体一致的完整名称。
///
/// v0.2.5 之前 `MODELS` 的 label 是 `Ultra-550b` 这类简称，且已随
/// `default_config()` 落进用户数据库；这里在加载时统一修正，
/// 使设置页与对话页对同一模型显示同一个名字。返回是否有改动。
fn migrate_model_labels(cfg: &mut RouterConfig) -> bool {
    let mut changed = false;
    for m in cfg.models.iter_mut() {
        for def in MODELS {
            if m.id == def.id && m.label != def.label {
                m.label = def.label.to_string();
                changed = true;
            }
        }
    }
    changed
}

pub fn load_config(conn: &Connection) -> RouterConfig {
    let mut cfg = match crate::db::get_setting(conn, CONFIG_KEY)
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
    };
    // 迁移旧简称后就地回写，避免每次启动都做一遍
    if migrate_model_labels(&mut cfg) {
        let _ = save_config(conn, &cfg);
    }
    cfg
}

pub fn save_config(conn: &Connection, cfg: &RouterConfig) -> Result<(), String> {
    let json = serde_json::to_string(cfg).map_err(|e| e.to_string())?;
    crate::db::set_setting(conn, CONFIG_KEY, &json).map_err(|e| e.to_string())
}

/// 合并供应商远端模型列表：已存在的（同 provider+id）不重复追加，
/// 本地已配置但远端缺失的条目保留（如离线时的快照）。返回新增数量。
///
/// `pinned` 控制新条目是否直接进降级链：`false` 表示仅作为可选目录，
/// 需用户勾选收藏后才生效；`true` 用于用户主动拉取的场景（如 Ollama）。
pub fn merge_remote_models(
    cfg: &mut RouterConfig,
    provider: &str,
    remote: Vec<RemoteModel>,
    tags: &[String],
    pinned: bool,
) -> usize {
    let mut added = 0;
    for m in remote {
        if cfg
            .models
            .iter()
            .any(|x| x.provider == provider && x.id == m.id)
        {
            continue;
        }
        cfg.models.push(ModelConfig {
            provider: provider.to_string(),
            id: m.id,
            label: m.label,
            context_limit: m.context_limit,
            enabled: true,
            pinned,
            tags: tags.to_vec(),
        });
        added += 1;
    }
    added
}

/// 同步某供应商的收藏状态：`pinned_keys` 为空表示全部取消收藏。
///
/// 仅作用于该供应商自己的模型；`pinned` 之外的字段（标签等）保持不变。
/// 返回收藏数量。
pub fn set_pinned(
    cfg: &mut RouterConfig,
    provider: &str,
    pinned_keys: &[String],
) -> usize {
    let pinned: std::collections::HashSet<&str> =
        pinned_keys.iter().map(|s| s.as_str()).collect();
    let mut count = 0;
    for m in cfg
        .models
        .iter_mut()
        .filter(|m| m.provider == provider)
    {
        m.pinned = pinned.contains(m.key().as_str());
        if m.pinned {
            count += 1;
        }
    }
    count
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
                && m.pinned
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
            // 全部取消收藏时保留原 selected 文本，仅作兜底展示，
            // 不强制指向某个未被收藏的模型
            .unwrap_or_else(|| cfg.selected.clone());
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
            pinned: true,
            tags: vec![],
        });
        cfg.selected = model_key("ghost", "m1");
        let cfg = normalize(cfg);
        assert_eq!(cfg.models.len(), 4);
        // provider 已不存在的 selected 应回退到首个可选项
        assert_eq!(cfg.selected, cfg.models[0].key());

        // 禁用 provider 后其模型保留（可再启用），selected 若已不可选则保留原值
        let mut cfg = default_config();
        cfg.selected = cfg.models[2].key();
        cfg.providers[0].enabled = false;
        let cfg = normalize(cfg);
        assert_eq!(cfg.models.len(), 4);
        // 没有可选项时不强制改写 selected（避免覆盖用户选择）
        assert_eq!(cfg.selected, cfg.models[2].key());
    }

    #[test]
    fn merge_remote_models_appends_new_keeps_local() {
        let mut cfg = default_config();
        cfg.providers.push(ProviderConfig {
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
        });
        let remote = vec![
            RemoteModel {
                id: "llama3:8b".into(),
                label: "Llama3 8B".into(),
                context_limit: 8192,
            },
            RemoteModel {
                id: "qwen3:0.6b".into(),
                label: "Qwen3 0.6B".into(),
                context_limit: 32768,
            },
        ];
        let added = merge_remote_models(&mut cfg, "ollama", remote, &["local".into()], true);
        assert_eq!(added, 2);
        assert_eq!(cfg.models.len(), MODELS.len() + 2);

        // 同 provider+id 重复注入不新增；远端缺失的本地条目保留
        let dup = vec![RemoteModel {
            id: "llama3:8b".into(),
            label: "Llama3 8B".into(),
            context_limit: 8192,
        }];
        let added = merge_remote_models(&mut cfg, "ollama", dup, &["local".into()], true);
        assert_eq!(added, 0);
        assert_eq!(cfg.models.len(), MODELS.len() + 2);

        // 同 id 不同 provider 视为不同模型（如 openrouter/qwen 与 ollama/qwen）
        let added = merge_remote_models(
            &mut cfg,
            "ollama",
            vec![RemoteModel {
                id: MODELS[0].id.into(),
                label: "dup".into(),
                context_limit: 1,
            }],
            &[],
            false,
        );
        assert_eq!(added, 1);

        let merged = cfg.models.iter().find(|m| m.id == "qwen3:0.6b").unwrap();
        assert_eq!(merged.provider, "ollama");
        assert_eq!(merged.tags, vec!["local".to_string()]);
        assert_eq!(merged.key(), "ollama:qwen3:0.6b");

        // normalize 后供应商仍在，模型保留
        let cfg = normalize(cfg);
        assert!(cfg.models.iter().any(|m| m.provider == "ollama"));
    }

    #[test]
    fn remote_models_start_unpinned_and_pin_is_idempotent() {
        let mut cfg = default_config();
        // 远端拉取的目录条目默认未收藏，不进降级链
        let added = merge_remote_models(
            &mut cfg,
            "openrouter",
            vec![
                RemoteModel {
                    id: "vendor/a".into(),
                    label: "A".into(),
                    context_limit: 8192,
                },
                RemoteModel {
                    id: "vendor/b".into(),
                    label: "B".into(),
                    context_limit: 8192,
                },
            ],
            &[],
            false,
        );
        assert_eq!(added, 2);
        let chain_keys: Vec<String> = cfg
            .models
            .iter()
            .filter(|m| m.pinned)
            .map(|m| m.key())
            .collect();
        assert!(!chain_keys.contains(&"openrouter:vendor/a".to_string()));

        // 勾选收藏：set_pinned 为幂等覆盖，仅保留传入的 key（默认 4 个模型被取消收藏）
        let n = set_pinned(
            &mut cfg,
            "openrouter",
            &[
                "openrouter:vendor/a".to_string(),
                "openrouter:vendor/b".to_string(),
            ],
        );
        assert_eq!(n, 2);
        assert_eq!(cfg.models.iter().filter(|m| m.pinned).count(), 2);
        // 幂等：重复提交结果一致
        set_pinned(&mut cfg, "openrouter", &["openrouter:vendor/a".to_string()]);
        assert_eq!(cfg.models.iter().filter(|m| m.pinned).count(), 1);

        // 取消全部收藏（只影响该 provider）
        assert_eq!(set_pinned(&mut cfg, "openrouter", &[]), 0);
        assert!(!cfg.models.iter().any(|m| m.pinned));
    }

    #[test]
    fn legacy_config_without_pinned_field_defaults_to_true() {
        // 模拟旧版本持久化的配置：models 数组没有 pinned 字段
        let json = r#"{
            "providers": [{
                "id": "openrouter", "kind": "openrouter", "name": "OpenRouter",
                "base_url": "https://openrouter.ai/api/v1", "enabled": true
            }],
            "models": [{
                "provider": "openrouter", "id": "legacy/model",
                "label": "Legacy", "context_limit": 8192, "enabled": true
            }],
            "selected": "openrouter:legacy/model"
        }"#;
        let cfg: RouterConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.models[0].pinned, "旧配置应默认视为已收藏");
        // selected 指向该模型，normalize 不应改写
        let cfg = normalize(cfg);
        assert_eq!(cfg.selected, "openrouter:legacy/model");
    }

    #[test]
    fn migrate_model_labels_refreshes_legacy_short_names() {
        let mut cfg = default_config();
        // 模拟旧版本落库的简称
        cfg.models[0].label = "Ultra-550b".into();
        cfg.models[1].label = "Super-120b".into();
        assert!(migrate_model_labels(&mut cfg));
        assert_eq!(cfg.models[0].label, "nemotron-3-ultra-550b-a55b");
        assert_eq!(cfg.models[1].label, "nemotron-3-super-120b-a12b");

        // 已是新名称则不再改动（幂等，避免每次启动都回写数据库）
        assert!(!migrate_model_labels(&mut cfg));
    }

    #[test]
    fn migrate_model_labels_leaves_other_models_untouched() {
        let mut cfg = default_config();
        cfg.models.push(ModelConfig {
            provider: "deepseek".into(),
            id: "deepseek-chat".into(),
            label: "DeepSeek V3".into(),
            context_limit: 65536,
            enabled: true,
            pinned: true,
            tags: vec![],
        });
        assert!(!migrate_model_labels(&mut cfg));
        assert_eq!(cfg.models[4].label, "DeepSeek V3");
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
