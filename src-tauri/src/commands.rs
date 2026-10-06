use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::Value;
use tauri::{AppHandle, State};
use tokio::sync::Mutex as AsyncMutex;

use crate::agent::runner::{run_turn, TurnContext};
use crate::api::{OpenRouterClient, MODELS};
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
    pub client: OpenRouterClient,
    pub sessions: AsyncMutex<HashMap<String, Session>>,
    pub workspace: Mutex<PathBuf>,
    pub global_rules: Mutex<String>,
    pub cancelled: AtomicBool,
    pub db: Mutex<Connection>,
    pub selected_model: Mutex<usize>,
}

impl AppState {
    pub fn new(api_key: Option<String>, db: Connection) -> Self {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let global_rules = db::get_setting(&db, "global_rules")
            .ok()
            .flatten()
            .unwrap_or_default();
        let selected_model = db::get_setting(&db, "model_id")
            .ok()
            .flatten()
            .and_then(|id| MODELS.iter().position(|m| m.id == id))
            .unwrap_or(0);
        Self {
            client: OpenRouterClient::new(api_key),
            sessions: AsyncMutex::new(HashMap::new()),
            workspace: Mutex::new(workspace),
            global_rules: Mutex::new(global_rules),
            cancelled: AtomicBool::new(false),
            db: Mutex::new(db),
            selected_model: Mutex::new(selected_model),
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
    if !state.client.has_api_key() {
        return Err("未配置 OPENROUTER_API_KEY 环境变量".into());
    }
    state.cancelled.store(false, Ordering::Relaxed);
    let workspace = state.workspace.lock().map_err(lock_err)?.clone();
    let global_rules = state.global_rules.lock().map_err(lock_err)?.clone();

    let mut sessions = state.sessions.lock().await;
    let session_id_for_load = session_id.clone();
    let default_model = state.selected_model.lock().map(|m| *m).unwrap_or(0);
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
            client: &state.client,
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
    *state.workspace.lock().map_err(lock_err)? = c.clone();
    Ok(c.display().to_string().replace('\\', "/"))
}

#[tauri::command]
pub fn get_workspace(state: State<'_, AppState>) -> String {
    state
        .workspace
        .lock()
        .map(|w| w.display().to_string().replace('\\', "/"))
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
    pub label: String,
    pub context_limit: usize,
}

#[tauri::command]
pub fn get_models() -> Vec<ModelInfo> {
    MODELS
        .iter()
        .map(|m| ModelInfo {
            id: m.id.to_string(),
            label: m.label.to_string(),
            context_limit: m.context_limit,
        })
        .collect()
}

#[tauri::command]
pub fn get_selected_model(state: State<'_, AppState>) -> String {
    let idx = state
        .selected_model
        .lock()
        .map(|m| *m)
        .unwrap_or(0)
        .min(MODELS.len() - 1);
    MODELS[idx].id.to_string()
}

#[tauri::command]
pub fn set_selected_model(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let pos = MODELS
        .iter()
        .position(|m| m.id == id)
        .ok_or_else(|| "未知模型".to_string())?;
    *state.selected_model.lock().map_err(lock_err)? = pos;
    if let Ok(mut sessions) = state.sessions.try_lock() {
        for session in sessions.values_mut() {
            session.model_idx = pos;
        }
    }
    let db = state.db.lock().map_err(lock_err)?;
    db::set_setting(&db, "model_id", &id).map_err(|e| e.to_string())
}
