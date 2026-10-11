use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde_json::Value;

pub struct SavedSession {
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub messages: Vec<Value>,
    pub todos: Vec<crate::agent::Todo>,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn json_err(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

pub fn open(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS sessions (
             id TEXT PRIMARY KEY,
             title TEXT NOT NULL,
             workspace TEXT NOT NULL,
             history_json TEXT NOT NULL DEFAULT '[]',
             model_idx INTEGER NOT NULL DEFAULT 0,
             todos_json TEXT NOT NULL DEFAULT '[]',
             updated_at INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS messages (
             session_id TEXT NOT NULL,
             seq INTEGER NOT NULL,
             payload TEXT NOT NULL,
             PRIMARY KEY (session_id, seq)
         );",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

/// 迁移历史脏数据：早期 set_workspace 曾写入带 //?/ 扩展前缀的键，
/// 与普通路径并存导致侧边栏出现重复的 src-tauri 分组。
/// 同时补齐 v0.4.0 新增的 `todos_json` 列。
pub(crate) fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE sessions SET workspace = replace(workspace, '//?/', '')
         WHERE workspace LIKE '//?/%'",
        [],
    )?;

    // CREATE TABLE IF NOT EXISTS 不会给已存在的表补列，老库必须单独 ALTER。
    // 用 pragma_table_info 判断而非「执行后再看报错」，因为 SQLite 不支持
    // IF NOT EXISTS 形式的 ADD COLUMN，重复执行会直接失败。
    let has_todos = conn
        .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'todos_json'")?
        .query([])?
        .next()?
        .is_some();
    if !has_todos {
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN todos_json TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    match rows.next()? {
        Some(row) => Ok(row.get(0)?),
        None => Ok(None),
    }
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn save_session(
    conn: &Connection,
    id: &str,
    title: &str,
    workspace: &str,
    messages: &[Value],
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO sessions (id, title, workspace, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
             title = excluded.title,
             workspace = excluded.workspace,
             updated_at = excluded.updated_at",
        params![id, title, workspace, now()],
    )?;
    conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id])?;
    let mut stmt =
        conn.prepare_cached("INSERT INTO messages (session_id, seq, payload) VALUES (?1, ?2, ?3)")?;
    for (i, m) in messages.iter().enumerate() {
        stmt.execute(params![id, i as i64, m.to_string()])?;
    }
    Ok(())
}

/// 仅更新 agent 历史，绝不插入会话行。
///
/// 历史行由前端 `save_session` 负责创建（携带真实 title/workspace）；
/// 这里若用 INSERT OR IGNORE 会写入 `title=''`、`workspace=''` 的脏行，
/// 导致下次启动侧边栏多出��空白工作区分组。
pub fn save_history(
    conn: &Connection,
    id: &str,
    history: &[Value],
    model_idx: usize,
) -> Result<(), rusqlite::Error> {
    let json = serde_json::to_string(history).map_err(json_err)?;
    conn.execute(
        "UPDATE sessions SET history_json = ?2, model_idx = ?3 WHERE id = ?1",
        params![id, json, model_idx as i64],
    )?;
    Ok(())
}

pub fn load_history(
    conn: &Connection,
    id: &str,
) -> Result<Option<(Vec<Value>, usize)>, rusqlite::Error> {
    let mut stmt =
        conn.prepare_cached("SELECT history_json, model_idx FROM sessions WHERE id = ?1")?;
    let mut rows = stmt.query(params![id])?;
    match rows.next()? {
        Some(row) => {
            let json: String = row.get(0)?;
            let idx: i64 = row.get(1)?;
            let history = serde_json::from_str(&json).map_err(json_err)?;
            Ok(Some((history, idx as usize)))
        }
        None => Ok(None),
    }
}

/// 覆盖保存任务清单，绝不插入会话行。
///
/// 与 [`save_history`] 同理：会话行由前端 `save_session` 创建，此处若 INSERT
/// 会写出 `title=''` 的脏行，让侧边栏多出空白工作区分组。
pub fn save_todos(
    conn: &Connection,
    id: &str,
    todos: &[crate::agent::Todo],
) -> Result<(), rusqlite::Error> {
    let json = serde_json::to_string(todos).map_err(json_err)?;
    conn.execute(
        "UPDATE sessions SET todos_json = ?2 WHERE id = ?1",
        params![id, json],
    )?;
    Ok(())
}

/// 读取任务清单。会话不存在或内容损坏时返回空清单而非报错——
/// 清单是增强信息，不该让整个会话加载失败。
pub fn load_todos(
    conn: &Connection,
    id: &str,
) -> Result<Vec<crate::agent::Todo>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached("SELECT todos_json FROM sessions WHERE id = ?1")?;
    let mut rows = stmt.query(params![id])?;
    let Some(row) = rows.next()? else {
        return Ok(Vec::new());
    };
    let json: String = row.get(0)?;
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

fn load_messages(conn: &Connection, session_id: &str) -> Result<Vec<Value>, rusqlite::Error> {
    let mut stmt =
        conn.prepare_cached("SELECT payload FROM messages WHERE session_id = ?1 ORDER BY seq")?;
    let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for payload in rows {
        out.push(serde_json::from_str(&payload?).map_err(json_err)?);
    }
    Ok(out)
}

pub fn load_sessions(conn: &Connection) -> Result<Vec<SavedSession>, rusqlite::Error> {
    let mut stmt =
        conn.prepare_cached("SELECT id, title, workspace FROM sessions ORDER BY updated_at DESC")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, title, workspace) = row?;
        let messages = load_messages(conn, &id)?;
        let todos = load_todos(conn, &id)?;
        out.push(SavedSession {
            id,
            title,
            workspace,
            messages,
            todos,
        });
    }
    Ok(out)
}

/// 物理删除空会话：没有任何消息，或工作区字段为空（历史脏数据）。
/// 返回被删除的行数。启动时调用一次，保证侧边栏不被空对话挤满。
pub fn purge_empty_sessions(conn: &Connection) -> Result<usize, rusqlite::Error> {
    conn.execute(
        "DELETE FROM messages WHERE session_id IN (
            SELECT s.id FROM sessions s
            LEFT JOIN messages m ON m.session_id = s.id
            WHERE s.workspace = '' OR m.session_id IS NULL
         )",
        [],
    )?;
    let removed = conn.execute(
        "DELETE FROM sessions WHERE workspace = ''
         OR id NOT IN (SELECT DISTINCT session_id FROM messages)",
        [],
    )?;
    Ok(removed)
}

pub fn delete_session(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id])?;
    conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn delete_workspace(conn: &Connection, workspace: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM messages WHERE session_id IN (SELECT id FROM sessions WHERE workspace = ?1)",
        params![workspace],
    )?;
    conn.execute("DELETE FROM sessions WHERE workspace = ?1", params![workspace])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_roundtrip_and_upsert() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "setting"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("t.db")).unwrap();

        assert_eq!(get_setting(&conn, "global_rules").unwrap(), None);
        set_setting(&conn, "global_rules", "使用中文回复").unwrap();
        assert_eq!(
            get_setting(&conn, "global_rules").unwrap().as_deref(),
            Some("使用中文回复")
        );
        set_setting(&conn, "global_rules", "使用英文回复").unwrap();
        assert_eq!(
            get_setting(&conn, "global_rules").unwrap().as_deref(),
            Some("使用英文回复")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "session"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("t.db")).unwrap();

        let messages = vec![
            serde_json::json!({ "id": "m1", "role": "user", "content": "你好" }),
            serde_json::json!({ "id": "m2", "role": "assistant", "blocks": [] }),
        ];
        save_session(&conn, "s1", "问候", "/ws", &messages).unwrap();
        let history = vec![serde_json::json!({ "role": "user", "content": "你好" })];
        save_history(&conn, "s1", &history, 1).unwrap();

        let loaded = load_sessions(&conn).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "s1");
        assert_eq!(loaded[0].title, "问候");
        assert_eq!(loaded[0].messages.len(), 2);

        let (restored, model_idx) = load_history(&conn, "s1").unwrap().unwrap();
        assert_eq!(restored, history);
        assert_eq!(model_idx, 1);

        save_session(&conn, "s1", "问候2", "/ws2", &messages[..1]).unwrap();
        let loaded = load_sessions(&conn).unwrap();
        assert_eq!(loaded[0].title, "问候2");
        assert_eq!(loaded[0].messages.len(), 1);
        let (_, model_idx) = load_history(&conn, "s1").unwrap().unwrap();
        assert_eq!(model_idx, 1, "保存 UI 消息不应清空 history");

        delete_session(&conn, "s1").unwrap();
        assert!(load_sessions(&conn).unwrap().is_empty());
        assert!(load_history(&conn, "s1").unwrap().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn purge_removes_empty_and_dirty_sessions_only() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "purge"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("t.db")).unwrap();

        let msg = serde_json::json!({ "id": "m1", "role": "user", "content": "hi" });
        // 有消息：保留
        save_session(&conn, "keep", "正常对话", "E:/dev/aura", &[msg]).unwrap();
        // 无消息：应清除
        save_session(&conn, "blank", "新对话", "E:/dev/aura", &[]).unwrap();
        // 模拟 save_history 历史脏行（workspace 为空）
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, title, workspace, updated_at) VALUES ('dirty','',' ', 0)",
            [],
        )
        .unwrap();

        assert_eq!(load_sessions(&conn).unwrap().len(), 3);
        let removed = purge_empty_sessions(&conn).unwrap();
        assert_eq!(removed, 2);

        let left = load_sessions(&conn).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "keep");

        // 幂等：再次清理不应删掉任何东西
        assert_eq!(purge_empty_sessions(&conn).unwrap(), 0);
        assert_eq!(load_sessions(&conn).unwrap().len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_history_never_creates_session_row() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "history"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("t.db")).unwrap();

        // 会话行不存在时，save_history 不应凭空插入空 title/workspace 的行
        save_history(&conn, "ghost", &[serde_json::json!({"role":"user"})], 0).unwrap();
        assert!(load_sessions(&conn).unwrap().is_empty());

        // 行存在时正常更新，且不破坏原有 title/workspace
        save_session(&conn, "s1", "标题", "E:/dev/aura", &[]).unwrap();
        save_history(&conn, "s1", &[serde_json::json!({"role":"user"})], 2).unwrap();
        let (hist, idx) = load_history(&conn, "s1").unwrap().unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(idx, 2);
        let loaded = load_sessions(&conn).unwrap();
        assert_eq!(loaded[0].title, "标题");
        assert_eq!(loaded[0].workspace, "E:/dev/aura");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_migrates_extended_workspace_keys() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "migration"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");

        let conn = open(&path).unwrap();
        save_session(&conn, "s1", "t", "//?/E:/dev/aura", &[]).unwrap();
        save_session(&conn, "s2", "t", "E:/dev/aura", &[]).unwrap();
        drop(conn);

        let conn = open(&path).unwrap();
        let loaded = load_sessions(&conn).unwrap();
        let ws1 = loaded
            .iter()
            .find(|s| s.id == "s1")
            .map(|s| s.workspace.clone());
        let ws2 = loaded
            .iter()
            .find(|s| s.id == "s2")
            .map(|s| s.workspace.clone());
        assert_eq!(ws1.as_deref(), Some("E:/dev/aura"));
        assert_eq!(ws2.as_deref(), Some("E:/dev/aura"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// v0.4.0 新增列。老库由 migrate() 的 ALTER TABLE 补齐，
    /// 这条测试刻意建一个不含 todos_json 的旧结构库来验证迁移。
    #[test]
    fn open_migrates_legacy_table_without_todos_column() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "legacy-todos"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.db");

        // 手工造一个 v0.3.x 结构的库：有会话行，但没有 todos_json 列
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 workspace TEXT NOT NULL,
                 history_json TEXT NOT NULL DEFAULT '[]',
                 model_idx INTEGER NOT NULL DEFAULT 0,
                 updated_at INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO sessions (id, title, workspace, updated_at)
             VALUES ('old', '旧会话', 'E:/dev/aura', 1);",
        )
        .unwrap();
        drop(conn);

        // 重新打开应完成迁移，且旧会话行不能丢
        let conn = open(&path).unwrap();
        let loaded = load_sessions(&conn).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "old");
        assert_eq!(loaded[0].title, "旧会话");
        assert!(loaded[0].todos.is_empty(), "旧会话默认空清单");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 迁移必须可重复执行：open() 每次启动都会跑 migrate()，
    /// 第二次运行若再 ALTER 一次会直接报错。
    #[test]
    fn migration_is_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "idempotent"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let conn = open(&path).unwrap();
        drop(conn);
        // 第二次 open 再次执行 migrate，不应因列已存在而失败
        let conn = open(&path).unwrap();
        drop(conn);
        let _ = open(&path).unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn todos_roundtrip_and_survive_message_saves() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "todos"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let conn = open(&path).unwrap();
        save_session(&conn, "s1", "t", "E:/dev/aura", &[]).unwrap();

        let todos = vec![
            crate::agent::Todo::new("s1", "建表"),
            crate::agent::Todo {
                id: "s2".into(),
                title: "写测试".into(),
                status: crate::agent::TodoStatus::Done,
            },
        ];
        save_todos(&conn, "s1", &todos).unwrap();

        // 保存 UI 消息（全量重写 messages）不应影响独立列
        save_session(
            &conn,
            "s1",
            "t",
            "E:/dev/aura",
            &[serde_json::json!({"id":"m1","role":"user","content":"hi"})],
        )
        .unwrap();

        let loaded = load_sessions(&conn).unwrap();
        let s = loaded.iter().find(|s| s.id == "s1").unwrap();
        assert_eq!(s.todos.len(), 2);
        assert_eq!(s.todos[0].title, "建表");
        // 关键：状态枚举必须原样往返，否则前端读到的是错的进度
        assert_eq!(s.todos[1].status, crate::agent::TodoStatus::Done);
        assert_eq!(s.messages.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_todos_on_missing_session_is_empty_not_error() {
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "no-session"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("t.db")).unwrap();
        assert!(load_todos(&conn, "nope").unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_todos_does_not_create_ghost_session() {
        // 会话行只由 save_session 创建；save_todos 若 INSERT 会写出
        // title='' 的脏行，让侧边栏多出空白工作区分组
        let dir = std::env::temp_dir().join(format!(
            "aura-db-test-{}-{}",
            std::process::id(),
            "ghost"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = open(&dir.join("t.db")).unwrap();
        save_todos(&conn, "ghost", &[crate::agent::Todo::new("s1", "任务")]).unwrap();
        assert!(load_sessions(&conn).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
