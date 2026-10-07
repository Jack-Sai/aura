pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub context_limit: usize,
}

/// v0.1.x 遗留的默认模型表：v0.2.0 起仅作为配置迁移时的出厂默认，
/// 运行时模型列表由 `config::RouterConfig` 驱动。
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

/// 降级命中的结果：携带实际使用的模型索引与提示文案。
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
    fn default_models_table_is_stable() {
        assert_eq!(MODELS.len(), 4);
        assert_eq!(MODELS[0].id, "nvidia/nemotron-3-ultra-550b-a55b:free");
        assert!(MODELS.iter().all(|m| m.context_limit >= 65536));
    }

    #[test]
    fn fallback_carries_notice_and_index() {
        let fb: Fallback<&str> = Fallback {
            value: "ok",
            model_idx: 2,
            notice: Some("主模型限流，已自动降级至 Super-120b".into()),
        };
        assert_eq!(fb.model_idx, 2);
        assert!(fb.notice.unwrap().contains("降级"));
        assert_eq!(fb.value, "ok");
        let _ = json_unused();
    }

    fn json_unused() -> Value {
        serde_json::Value::Null
    }
}
