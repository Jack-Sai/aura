use std::path::PathBuf;
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

pub struct AppState {
    pub client: OpenRouterClient,
    pub session: AsyncMutex<Session>,
    pub workspace: Mutex<PathBuf>,
    pub global_rules: Mutex<String>,
}

impl AppState {
    pub fn new(api_key: Option<String>) -> Self {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            client: OpenRouterClient::new(api_key),
            session: AsyncMutex::new(Session {
                history: Vec::new(),
                model_idx: 0,
            }),
            workspace: Mutex::new(workspace),
            global_rules: Mutex::new(String::new()),
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
    message: String,
) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("消息不能为空".into());
    }
    if !state.client.has_api_key() {
        return Err("未配置 OPENROUTER_API_KEY 环境变量".into());
    }
    let workspace = state.workspace.lock().map_err(lock_err)?.clone();
    let global_rules = state.global_rules.lock().map_err(lock_err)?.clone();

    let mut session = state.session.lock().await;
    let session = &mut *session;
    let mut ctx = TurnContext {
        app: &app,
        client: &state.client,
        workspace: &workspace,
        global_rules: &global_rules,
        history: &mut session.history,
        model_idx: &mut session.model_idx,
    };
    run_turn(&mut ctx, message).await
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
