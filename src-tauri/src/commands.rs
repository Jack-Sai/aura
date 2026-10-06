use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::Value;
use tauri::{AppHandle, State};
use tokio::sync::Mutex as AsyncMutex;

use crate::agent::runner::{run_turn, TurnContext};
use crate::api::OpenRouterClient;

pub struct Session {
    pub history: Vec<Value>,
    pub model_idx: usize,
}

impl Session {
    fn new() -> Self {
        Self {
            history: Vec::new(),
            model_idx: 0,
        }
    }
}

pub struct AppState {
    pub client: OpenRouterClient,
    pub sessions: AsyncMutex<HashMap<String, Session>>,
    pub workspace: Mutex<PathBuf>,
    pub global_rules: Mutex<String>,
    pub cancelled: AtomicBool,
}

impl AppState {
    pub fn new(api_key: Option<String>) -> Self {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            client: OpenRouterClient::new(api_key),
            sessions: AsyncMutex::new(HashMap::new()),
            workspace: Mutex::new(workspace),
            global_rules: Mutex::new(String::new()),
            cancelled: AtomicBool::new(false),
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
    let session = sessions
        .entry(session_id.clone())
        .or_insert_with(Session::new);
    let session = &mut *session;
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
}

#[tauri::command]
pub fn stop_message(state: State<'_, AppState>) {
    state.cancelled.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn remove_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    state.sessions.lock().await.remove(&session_id);
    Ok(())
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
