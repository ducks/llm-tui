use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::PathBuf;

use crate::session::{Message, Session};

pub fn get_db_path() -> Result<PathBuf> {
    let mut path = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find local data directory"))?;
    path.push("llm-tui");
    std::fs::create_dir_all(&path)?;
    path.push("sessions.db");
    Ok(path)
}

pub fn init_db() -> Result<Connection> {
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path)?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            name TEXT,
            project TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            llm_provider TEXT NOT NULL,
            model TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            model TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id)
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC)",
        [],
    )?;

    // Migration: Add model column to sessions if it doesn't exist
    let sessions_has_model: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name='model'")?
        .query_row([], |row| row.get(0))
        .map(|count: i32| count > 0)?;

    if !sessions_has_model {
        conn.execute("ALTER TABLE sessions ADD COLUMN model TEXT", [])?;
    }

    // Migration: Add model column to messages if it doesn't exist
    let messages_has_model: bool = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='model'")?
        .query_row([], |row| row.get(0))
        .map(|count: i32| count > 0)?;

    if !messages_has_model {
        conn.execute("ALTER TABLE messages ADD COLUMN model TEXT", [])?;
    }

    Ok(conn)
}

pub fn save_session(conn: &Connection, session: &Session) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sessions (id, name, project, created_at, updated_at, llm_provider, model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            session.id,
            session.name,
            session.project,
            session.created_at.timestamp(),
            session.updated_at.timestamp(),
            session.llm_provider,
            session.model,
        ],
    )?;
    Ok(())
}

pub fn save_message(conn: &Connection, session_id: &str, message: &Message) -> Result<()> {
    conn.execute(
        "INSERT INTO messages (session_id, role, content, timestamp, model)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            session_id,
            message.role,
            message.content,
            message.timestamp.timestamp(),
            message.model,
        ],
    )?;
    Ok(())
}

pub fn load_session(conn: &Connection, session_id: &str) -> Result<Session> {
    let mut stmt = conn.prepare(
        "SELECT id, name, project, created_at, updated_at, llm_provider, model
         FROM sessions WHERE id = ?1"
    )?;

    let session = stmt.query_row([session_id], |row| {
        Ok(Session {
            id: row.get(0)?,
            name: row.get(1)?,
            project: row.get(2)?,
            created_at: chrono::DateTime::from_timestamp(row.get(3)?, 0)
                .unwrap_or_else(|| chrono::Utc::now()),
            updated_at: chrono::DateTime::from_timestamp(row.get(4)?, 0)
                .unwrap_or_else(|| chrono::Utc::now()),
            llm_provider: row.get(5)?,
            model: row.get(6)?,
            messages: Vec::new(),
        })
    })?;

    Ok(session)
}

pub fn load_messages(conn: &Connection, session_id: &str) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT role, content, timestamp, model FROM messages
         WHERE session_id = ?1 ORDER BY timestamp ASC"
    )?;

    let messages = stmt.query_map([session_id], |row| {
        Ok(Message {
            role: row.get(0)?,
            content: row.get(1)?,
            timestamp: chrono::DateTime::from_timestamp(row.get(2)?, 0)
                .unwrap_or_else(|| chrono::Utc::now()),
            model: row.get(3)?,
            tools_executed: false, // Old messages from DB default to false
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(messages)
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<Session>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, project, created_at, updated_at, llm_provider, model
         FROM sessions ORDER BY updated_at DESC"
    )?;

    let sessions = stmt.query_map([], |row| {
        Ok(Session {
            id: row.get(0)?,
            name: row.get(1)?,
            project: row.get(2)?,
            created_at: chrono::DateTime::from_timestamp(row.get(3)?, 0)
                .unwrap_or_else(|| chrono::Utc::now()),
            updated_at: chrono::DateTime::from_timestamp(row.get(4)?, 0)
                .unwrap_or_else(|| chrono::Utc::now()),
            llm_provider: row.get(5)?,
            model: row.get(6)?,
            messages: Vec::new(),
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(sessions)
}

pub fn delete_session(conn: &Connection, session_id: &str) -> Result<()> {
    // Delete messages first (foreign key)
    conn.execute(
        "DELETE FROM messages WHERE session_id = ?1",
        [session_id],
    )?;

    // Delete session
    conn.execute(
        "DELETE FROM sessions WHERE id = ?1",
        [session_id],
    )?;

    Ok(())
}

pub fn rename_session(conn: &Connection, session_id: &str, new_name: &str) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET name = ?1 WHERE id = ?2",
        [new_name, session_id],
    )?;
    Ok(())
}
