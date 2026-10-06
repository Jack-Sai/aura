use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde_json::Value;

pub struct SavedSession {
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub messages: Vec<Value>,
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
             updated_at INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS messages (
             session_id TEXT NOT NULL,
             seq INTEGER NOT NULL,
             payload TEXT NOT NULL,
             PRIMARY KEY (session_id, seq)
         );",
    )?;
    Ok(conn)
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

pub fn save_history(
    conn: &Connection,
    id: &str,
    history: &[Value],
    model_idx: usize,
) -> Result<(), rusqlite::Error> {
    let json = serde_json::to_string(history).map_err(json_err)?;
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id, title, workspace, updated_at) VALUES (?1, '', '', 0)",
        params![id],
    )?;
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
        out.push(SavedSession {
            id,
            title,
            workspace,
            messages,
        });
    }
    Ok(out)
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
}
