use rusqlite::{params, Connection};
use codiv_common::conversation::{ConversationEvent, SessionInfo};
use tracing::{info, warn};

/// SQLite-backed session store.
///
/// All database access is synchronous (rusqlite `Connection` is `!Sync`),
/// so the store lives on the daemon's main task and is never shared across
/// spawn boundaries.
pub struct SessionStore {
    conn: Connection,
}

impl SessionStore {
    /// Open (or create) the database at `~/.codiv/sessions.db` and
    /// run any pending migrations.
    pub fn open() -> Result<Self, rusqlite::Error> {
        let dir = codiv_common::config::config_dir();
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("sessions.db");
        let conn = Connection::open(&path)?;
        let store = Self { conn };
        store.migrate()?;
        info!("session store opened at {}", path.display());
        Ok(store)
    }

    fn migrate(&self) -> Result<(), rusqlite::Error> {
        self.conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;

            CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                name        TEXT,
                parent_id   TEXT,
                fork_at_seq INTEGER,
                cwd         TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT NOT NULL REFERENCES sessions(id),
                seq         INTEGER NOT NULL,
                event_type  TEXT NOT NULL,
                payload     TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(session_id, seq)
            );

            CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, seq);
            ",
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Session CRUD
    // ------------------------------------------------------------------

    pub fn create_session(&self, id: &str, cwd: &str) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO sessions (id, cwd) VALUES (?1, ?2)",
            params![id, cwd],
        )?;
        Ok(())
    }

    pub fn update_session_name(&self, id: &str, name: &str) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE sessions SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    /// Look up a single session's name by its ID.
    pub fn get_session_name_by_id(&self, id: &str) -> Result<Option<String>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], |row| row.get::<_, Option<String>>(0))?;
        match rows.next() {
            Some(Ok(name)) => Ok(name),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionInfo>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, parent_id, cwd, created_at, updated_at
             FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SessionInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                cwd: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    // ------------------------------------------------------------------
    // Event persistence
    // ------------------------------------------------------------------

    /// Append a single conversation event.  `seq` must be monotonically
    /// increasing per session.
    pub fn append_event(
        &self,
        session_id: &str,
        seq: u32,
        event: &ConversationEvent,
    ) -> Result<(), rusqlite::Error> {
        let event_type = event_discriminant(event);
        let payload = match serde_json::to_string(event) {
            Ok(p) => p,
            Err(e) => {
                warn!(seq, %e, "failed to serialize conversation event");
                return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
            }
        };
        self.conn.execute(
            "INSERT INTO events (session_id, seq, event_type, payload) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, seq, event_type, payload],
        )?;
        // Touch the session's updated_at timestamp.
        self.conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Load all events for a session, optionally up to (inclusive) a given seq.
    pub fn load_events(
        &self,
        session_id: &str,
        up_to_seq: Option<u32>,
    ) -> Result<Vec<ConversationEvent>, rusqlite::Error> {
        let (sql, limit) = match up_to_seq {
            Some(max) => (
                "SELECT payload FROM events WHERE session_id = ?1 AND seq <= ?2 ORDER BY seq",
                max as i64,
            ),
            None => (
                "SELECT payload FROM events WHERE session_id = ?1 AND seq <= ?2 ORDER BY seq",
                i64::MAX,
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![session_id, limit], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        })?;
        let mut events = Vec::new();
        for (seq_counter, row) in (0_i64..).zip(rows) {
            let json = row?;
            match serde_json::from_str::<ConversationEvent>(&json) {
                Ok(ev) => events.push(ev),
                Err(e) => {
                    warn!(seq = seq_counter, %e, "failed to deserialize event, skipping");
                }
            }
        }
        Ok(events)
    }

}

/// Map a `ConversationEvent` variant to a string tag for the `event_type` column.
fn event_discriminant(event: &ConversationEvent) -> &'static str {
    match event {
        ConversationEvent::UserPrompt { .. } => "user_prompt",
        ConversationEvent::ShellCommand { .. } => "shell_command",
        ConversationEvent::AssistantText { .. } => "assistant_text",
        ConversationEvent::AssistantReasoning { .. } => "assistant_reasoning",
        ConversationEvent::ToolCall { .. } => "tool_call",
        ConversationEvent::ToolResult { .. } => "tool_result",
        ConversationEvent::Error { .. } => "error",
        ConversationEvent::TokenUsage { .. } => "token_usage",
        ConversationEvent::Summary { .. } => "summary",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> SessionStore {
        let conn = Connection::open_in_memory().unwrap();
        let store = SessionStore { conn };
        store.migrate().unwrap();
        store
    }

    #[test]
    fn create_and_list() {
        let store = mem_store();
        store.create_session("s1", "/tmp").unwrap();
        store.update_session_name("s1", "hello world").unwrap();
        let sessions = store.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(sessions[0].name.as_deref(), Some("hello world"));
    }

    #[test]
    fn get_session_name_by_id() {
        let store = mem_store();
        store.create_session("s1", "/tmp").unwrap();
        // No name set yet — should return Ok(None)
        assert_eq!(store.get_session_name_by_id("s1").unwrap(), None);
        // Set a name and verify
        store.update_session_name("s1", "my session").unwrap();
        assert_eq!(
            store.get_session_name_by_id("s1").unwrap(),
            Some("my session".to_string())
        );
        // Non-existent session returns Ok(None)
        assert_eq!(store.get_session_name_by_id("nonexistent").unwrap(), None);
    }

    #[test]
    fn append_and_load_events() {
        let store = mem_store();
        store.create_session("s1", "/tmp").unwrap();
        let ev1 = ConversationEvent::UserPrompt {
            text: "hi".into(),
            request_id: "r1".into(),
        };
        let ev2 = ConversationEvent::AssistantText {
            request_id: "r1".into(),
            text: "hello".into(),
        };
        store.append_event("s1", 0, &ev1).unwrap();
        store.append_event("s1", 1, &ev2).unwrap();
        let loaded = store.load_events("s1", None).unwrap();
        assert_eq!(loaded.len(), 2);
    }

}
