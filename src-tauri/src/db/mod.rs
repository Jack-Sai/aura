use std::path::Path;

use rusqlite::{params, Connection};

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
}
