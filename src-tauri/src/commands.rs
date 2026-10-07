use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::Value;
use tauri::{AppHandle, State};
use tokio::sync::Mutex as AsyncMutex;

use crate::agent::runner::{run_turn, TurnContext};
use crate::api::config::{load_config, normalize, save_config};
use crate::api::{ProviderConfig, Router};
use crate::db;

pub struct Session {
    pub history: Vec<Value>,
    pub model_idx: usize,
}

impl Session {
    fn new(model_idx: usize) -> Self {
        Self {
            history: Vec::new(),
            model_idx,
        }
    }
}

pub struct AppState {
    pub router: Router,
    pub sessions: AsyncMutex<HashMap<String, Session>>,
    pub workspace: Mutex<PathBuf>,
    pub global_rules: Mutex<String>,
    pub cancelled: AtomicBool,
    pub db: Mutex<Connection>,
}

/// Windows 上 `canonicalize()` 会返回 `\\?\` 扩展前缀路径，若与
/// `current_dir()` 等普通路径混用，同一目录会产生两种工作区键，
/// 导致侧边栏出现重复分组、会话错挂。统一剥离前缀并转为 `/` 分隔。
pub fn strip_extended_prefix(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

pub fn normalize_workspace_path(p: &std::path::Path) -> String {
    strip_extended_prefix(&p.display().to_string()).replace('\\', "/")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

impl AppState {
    pub fn new(env_api_key: Option<String>, db: Connection) -> Self {
        // 优先恢复上次使用的工作区；无记录（首次运行）时回退到用户主目录，
        // 避免以进程启动目录（如 src-tauri）作为工作区凭空出现
        let workspace = db::get_setting(&db, "last_workspace")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(home_dir)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let global_rules = db::get_setting(&db, "global_rules")
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut config = load_config(&db);
        // 环境变量作为 OpenRouter API Key 兜底（配置为空时）
        if let Some(env_key) = env_api_key {
            if let Some(p) = config.providers.iter_mut().find(|p| p.id == "openrouter") {
                if p.api_key.trim().is_empty() {
                    p.api_key = env_key;
                }
            }
        }
        let router = Router::new(config);
        Self {
            router,
            sessions: AsyncMutex::new(HashMap::new()),
            workspace: Mutex::new(workspace),
            global_rules: Mutex::new(global_rules),
            cancelled: AtomicBool::new(false),
            db: Mutex::new(db),
        }
    }
}

fn lock_err<T>(_: T) -> String {
    "内部状态锁损坏".to_string()
}

#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    message: String,
) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("消息不能为空".into());
    }
    if !state.router.has_credentials() {
        return Err("未配置模型服务凭据，请在设置中填写 API Key".into());
    }
    state.cancelled.store(false, Ordering::Relaxed);
    let workspace = state.workspace.lock().map_err(lock_err)?.clone();
    let global_rules = state.global_rules.lock().map_err(lock_err)?.clone();

    let mut sessions = state.sessions.lock().await;
    let session_id_for_load = session_id.clone();
    let default_model = state.router.selected_index();
    let session = sessions.entry(session_id.clone()).or_insert_with(|| {
        state
            .db
            .lock()
            .ok()
            .and_then(|db| db::load_history(&db, &session_id_for_load).ok())
            .flatten()
            .map(|(history, model_idx)| Session { history, model_idx })
            .unwrap_or_else(|| Session::new(default_model))
    });
    let session = &mut *session;
    let result = {
        let mut ctx = TurnContext {
            app: &app,
            router: &state.router,
            workspace: &workspace,
            global_rules: &global_rules,
            history: &mut session.history,
            model_idx: &mut session.model_idx,
            session_id: &session_id,
            cancelled: &state.cancelled,
        };
        run_turn(&mut ctx, message).await
    };
    if let Ok(db) = state.db.lock() {
        let _ = db::save_history(&db, &session_id, &session.history, session.model_idx);
    }
    result
}

#[tauri::command]
pub fn stop_message(state: State<'_, AppState>) {
    state.cancelled.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn remove_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    state.sessions.lock().await.remove(&session_id);
    let db = state.db.lock().map_err(lock_err)?;
    db::delete_session(&db, &session_id).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub messages: Vec<Value>,
}

#[tauri::command]
pub fn load_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSnapshot>, String> {
    let db = state.db.lock().map_err(lock_err)?;
    let saved = db::load_sessions(&db).map_err(|e| e.to_string())?;
    Ok(saved
        .into_iter()
        .map(|s| SessionSnapshot {
            id: s.id,
            title: s.title,
            workspace: s.workspace,
            messages: s.messages,
        })
        .collect())
}

#[tauri::command]
pub fn save_session(
    state: State<'_, AppState>,
    id: String,
    title: String,
    workspace: String,
    messages: Vec<Value>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(lock_err)?;
    db::save_session(&db, &id, &title, &workspace, &messages).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_workspace(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let p = PathBuf::from(path.trim());
    let meta = std::fs::metadata(&p).map_err(|_| "路径不存在".to_string())?;
    if !meta.is_dir() {
        return Err("不是目录".into());
    }
    let c = p.canonicalize().map_err(|e| e.to_string())?;
    let normalized = normalize_workspace_path(&c);
    {
        let db = state.db.lock().map_err(lock_err)?;
        let _ = db::set_setting(&db, "last_workspace", &normalized);
    }
    *state.workspace.lock().map_err(lock_err)? = c;
    Ok(normalized)
}

#[tauri::command]
pub fn get_workspace(state: State<'_, AppState>) -> String {
    state
        .workspace
        .lock()
        .map(|w| normalize_workspace_path(&w))
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_global_rules(state: State<'_, AppState>) -> Result<String, String> {
    state
        .global_rules
        .lock()
        .map(|r| r.clone())
        .map_err(lock_err)
}

#[tauri::command]
pub fn set_global_rules(state: State<'_, AppState>, rules: String) -> Result<(), String> {
    {
        let mut g = state.global_rules.lock().map_err(lock_err)?;
        *g = rules.clone();
    }
    let db = state.db.lock().map_err(lock_err)?;
    db::set_setting(&db, "global_rules", &rules).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    /// `{provider}:{id}` 复合 key（前端选中/切换主键）
    pub key: String,
    pub provider: String,
    pub label: String,
    pub context_limit: usize,
}

#[tauri::command]
pub fn get_models(state: State<'_, AppState>) -> Vec<ModelInfo> {
    let cfg = state.router.config();
    cfg.models
        .iter()
        .filter(|m| m.enabled)
        .map(|m| ModelInfo {
            id: m.id.clone(),
            key: m.key(),
            provider: m.provider.clone(),
            label: m.label.clone(),
            context_limit: m.context_limit,
        })
        .collect()
}

#[tauri::command]
pub fn get_selected_model(state: State<'_, AppState>) -> String {
    state.router.config().selected
}

#[tauri::command]
pub fn set_selected_model(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.router.config();
    // 兼容复合 key 与 v0.1.x 裸 id 两种入参
    let pos = cfg
        .models
        .iter()
        .position(|m| m.key() == id)
        .or_else(|| cfg.models.iter().position(|m| m.id == id))
        .ok_or_else(|| "未知模型".to_string())?;
    cfg.selected = cfg.models[pos].key();
    state.router.set_config(cfg.clone());
    {
        let db = state.db.lock().map_err(lock_err)?;
        save_config(&db, &cfg)?;
    }
    if let Ok(mut sessions) = state.sessions.try_lock() {
        for session in sessions.values_mut() {
            session.model_idx = pos;
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
pub struct ApiConfig {
    pub provider: String,
    pub api_key: String,
}

/// 返回全部供应商配置（设置页编辑用）。
#[tauri::command]
pub fn get_providers(state: State<'_, AppState>) -> Vec<ProviderConfig> {
    state.router.config().providers
}

/// 校验并整体保存供应商列表，重建 Router 执行器。
#[tauri::command]
pub fn set_providers(
    state: State<'_, AppState>,
    providers: Vec<ProviderConfig>,
) -> Result<(), String> {
    if providers.is_empty() {
        return Err("至少需要保留一个模型供应商".into());
    }
    let mut seen = std::collections::HashSet::new();
    for p in &providers {
        if p.id.trim().is_empty() || p.id.contains(':') || p.id.contains(char::is_whitespace) {
            return Err(format!("供应商 ID 非法（不可为空/含冒号或空格）：{}", p.id));
        }
        if !seen.insert(p.id.clone()) {
            return Err(format!("供应商 ID 重复：{}", p.id));
        }
        if p.name.trim().is_empty() {
            return Err(format!("供应商名称不能为空：{}", p.id));
        }
        if p.base_url.trim().is_empty() {
            return Err(format!("{} 的接口地址不能为空", p.name));
        }
        if p.kind == crate::api::ProviderKind::Azure
            && p.deployment.as_deref().unwrap_or("").trim().is_empty()
        {
            return Err(format!("{} 需要填写 Azure Deployment 名", p.name));
        }
    }
    let mut cfg = state.router.config();
    cfg.providers = providers;
    let cfg = normalize(cfg);
    state.router.set_config(cfg.clone());
    let db = state.db.lock().map_err(lock_err)?;
    save_config(&db, &cfg)
}

#[tauri::command]
pub fn get_api_config(state: State<'_, AppState>) -> ApiConfig {
    let cfg = state.router.config();
    ApiConfig {
        provider: "openrouter".to_string(),
        api_key: cfg
            .providers
            .iter()
            .find(|p| p.id == "openrouter")
            .map(|p| p.api_key.clone())
            .unwrap_or_default(),
    }
}

#[tauri::command]
pub fn set_api_config(
    state: State<'_, AppState>,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    if provider != "openrouter" {
        return Err("暂只支持 OpenRouter".to_string());
    }
    let key = api_key.trim().to_string();
    let effective = if key.is_empty() {
        std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
    } else {
        Some(key.clone())
    };
    state.router.set_provider_key("openrouter", effective);
    let mut cfg = state.router.config();
    if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == "openrouter") {
        // 持久化原始输入（空串=清除）；环境变量仅作运行时兜底
        p.api_key = key;
    }
    let db = state.db.lock().map_err(lock_err)?;
    save_config(&db, &cfg)
}

#[tauri::command]
pub async fn remove_workspace(
    state: State<'_, AppState>,
    path: String,
    session_ids: Vec<String>,
) -> Result<(), String> {
    {
        let mut sessions = state.sessions.lock().await;
        for id in &session_ids {
            sessions.remove(id);
        }
    }
    let db = state.db.lock().map_err(lock_err)?;
    db::delete_workspace(&db, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn strip_extended_prefix_plain() {
        assert_eq!(strip_extended_prefix("E:/dev/aura"), "E:/dev/aura");
        assert_eq!(strip_extended_prefix("E:\\dev\\aura"), "E:\\dev\\aura");
    }

    #[test]
    fn strip_extended_prefix_extended() {
        assert_eq!(
            strip_extended_prefix(r"\\?\E:\dev\aura"),
            r"E:\dev\aura"
        );
        assert_eq!(
            strip_extended_prefix(r"\\?\UNC\server\share\dir"),
            r"\\server\share\dir"
        );
    }

    #[test]
    fn normalize_workspace_path_backslash_and_extended() {
        assert_eq!(
            normalize_workspace_path(Path::new(r"\\?\E:\dev\aura")),
            "E:/dev/aura"
        );
        assert_eq!(
            normalize_workspace_path(Path::new(r"E:\dev\aura")),
            "E:/dev/aura"
        );
    }
}
