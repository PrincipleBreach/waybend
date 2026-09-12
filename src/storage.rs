use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::model::EvidenceEvent;

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("stored event {event_id} has an invalid timestamp: {source}")]
    Timestamp {
        event_id: String,
        source: chrono::ParseError,
    },
    #[error("stored event {event_id} has invalid headers: {source}")]
    Headers {
        event_id: String,
        source: serde_json::Error,
    },
    #[error("database lock is poisoned")]
    Poisoned,
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    pub kind: Option<String>,
    pub token: Option<String>,
    pub route_id: Option<String>,
    pub search: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

impl EventQuery {
    pub fn normalized(mut self) -> Self {
        self.limit = self.limit.clamp(1, 500);
        self
    }
}

#[derive(Debug, Clone)]
pub struct EvidenceStore {
    connection: Arc<Mutex<Connection>>,
}

pub type Store = EvidenceStore;

impl EvidenceStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn memory() -> Result<Self> {
        Self::in_memory()
    }

    fn from_connection(mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_metadata (
                version INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS evidence_events (
                id          TEXT PRIMARY KEY NOT NULL,
                received_at TEXT NOT NULL,
                kind        TEXT NOT NULL,
                token       TEXT,
                route_id    TEXT,
                source_ip   TEXT NOT NULL,
                method      TEXT,
                path        TEXT,
                query       TEXT,
                headers     TEXT NOT NULL,
                body        TEXT,
                dns_name    TEXT,
                dns_type    TEXT,
                dns_answer  TEXT,
                dns_sequence INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_evidence_received_at
                ON evidence_events(received_at DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_evidence_token
                ON evidence_events(token);
            CREATE INDEX IF NOT EXISTS idx_evidence_route_id
                ON evidence_events(route_id);
            CREATE INDEX IF NOT EXISTS idx_evidence_kind
                ON evidence_events(kind);",
        )?;
        let version: Option<i64> = connection
            .query_row("SELECT version FROM schema_metadata LIMIT 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        match version {
            None => {
                connection.execute(
                    "INSERT INTO schema_metadata(version) VALUES (?1)",
                    [SCHEMA_VERSION],
                )?;
            }
            Some(1) => {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                if !column_exists(&transaction, "dns_answer")? {
                    transaction
                        .execute("ALTER TABLE evidence_events ADD COLUMN dns_answer TEXT", [])?;
                }
                if !column_exists(&transaction, "dns_sequence")? {
                    transaction.execute(
                        "ALTER TABLE evidence_events ADD COLUMN dns_sequence INTEGER",
                        [],
                    )?;
                }
                transaction.execute("UPDATE schema_metadata SET version = 2", [])?;
                transaction.commit()?;
            }
            Some(found) if found > SCHEMA_VERSION => {
                return Err(StorageError::Database(rusqlite::Error::InvalidQuery));
            }
            _ => {}
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| StorageError::Poisoned)
    }

    pub fn insert(&self, event: &EvidenceEvent) -> Result<()> {
        let headers = serde_json::to_string(&event.headers).expect("map serialization cannot fail");
        self.connection()?.execute(
            "INSERT INTO evidence_events (
                id, received_at, kind, token, route_id, source_ip, method, path,
                query, headers, body, dns_name, dns_type, dns_answer, dns_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                event.id,
                event.received_at.to_rfc3339(),
                event.kind,
                event.token,
                event.route_id,
                event.source_ip,
                event.method,
                event.path,
                event.query,
                headers,
                event.body,
                event.dns_name,
                event.dns_type,
                event.dns_answer,
                event.dns_sequence,
            ],
        )?;
        Ok(())
    }

    pub fn record(&self, event: EvidenceEvent) -> Result<()> {
        self.insert(&event)
    }

    pub fn get(&self, id: &str) -> Result<Option<EvidenceEvent>> {
        let connection = self.connection()?;
        let raw = connection
            .query_row(
                "SELECT id, received_at, kind, token, route_id, source_ip, method,
                        path, query, headers, body, dns_name, dns_type, dns_answer, dns_sequence
                 FROM evidence_events WHERE id = ?1",
                [id],
                RawEvent::from_row,
            )
            .optional()?;
        raw.map(RawEvent::try_into_event).transpose()
    }

    pub fn list(&self, query: &EventQuery) -> Result<Vec<EvidenceEvent>> {
        let query = query.clone().normalized();
        let search = query.search.as_ref().map(|value| format!("%{value}%"));
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, received_at, kind, token, route_id, source_ip, method,
                    path, query, headers, body, dns_name, dns_type, dns_answer, dns_sequence
             FROM evidence_events
             WHERE (?1 IS NULL OR kind = ?1)
               AND (?2 IS NULL OR token = ?2)
               AND (?3 IS NULL OR route_id = ?3)
               AND (?4 IS NULL OR id LIKE ?4 OR source_ip LIKE ?4 OR
                    COALESCE(path, '') LIKE ?4 OR COALESCE(body, '') LIKE ?4 OR
                    COALESCE(dns_name, '') LIKE ?4 OR COALESCE(token, '') LIKE ?4 OR
                    COALESCE(route_id, '') LIKE ?4 OR COALESCE(query, '') LIKE ?4 OR
                    COALESCE(headers, '') LIKE ?4 OR COALESCE(dns_answer, '') LIKE ?4)
             ORDER BY received_at DESC, id DESC
             LIMIT ?5 OFFSET ?6",
        )?;
        let rows = statement.query_map(
            params![
                query.kind,
                query.token,
                query.route_id,
                search,
                query.limit as i64,
                query.offset as i64
            ],
            RawEvent::from_row,
        )?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row?.try_into_event()?);
        }
        Ok(events)
    }

    pub fn list_recent(&self, limit: usize, token: Option<&str>) -> Result<Vec<EvidenceEvent>> {
        self.list(&EventQuery {
            token: token.map(str::to_owned),
            limit,
            ..EventQuery::default()
        })
    }

    pub fn count(&self) -> Result<u64> {
        let count: i64 =
            self.connection()?
                .query_row("SELECT COUNT(*) FROM evidence_events", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    pub fn count_filtered(&self, query: &EventQuery) -> Result<u64> {
        let search = query.search.as_ref().map(|value| format!("%{value}%"));
        let count: i64 = self.connection()?.query_row(
            "SELECT COUNT(*) FROM evidence_events
             WHERE (?1 IS NULL OR kind = ?1)
               AND (?2 IS NULL OR token = ?2)
               AND (?3 IS NULL OR route_id = ?3)
               AND (?4 IS NULL OR id LIKE ?4 OR source_ip LIKE ?4 OR
                    COALESCE(path, '') LIKE ?4 OR COALESCE(body, '') LIKE ?4 OR
                    COALESCE(dns_name, '') LIKE ?4 OR COALESCE(token, '') LIKE ?4 OR
                    COALESCE(route_id, '') LIKE ?4 OR COALESCE(query, '') LIKE ?4 OR
                    COALESCE(headers, '') LIKE ?4 OR COALESCE(dns_answer, '') LIKE ?4)",
            params![query.kind, query.token, query.route_id, search],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    pub fn prune_older_than(&self, cutoff: DateTime<Utc>) -> Result<usize> {
        Ok(self.connection()?.execute(
            "DELETE FROM evidence_events WHERE received_at < ?1",
            [cutoff.to_rfc3339()],
        )?)
    }

    pub fn prune(&self, retention_days: u32) -> Result<usize> {
        self.prune_older_than(Utc::now() - chrono::Duration::days(i64::from(retention_days)))
    }

    pub fn prune_to_limit(&self, maximum: usize) -> Result<usize> {
        Ok(self.connection()?.execute(
            "DELETE FROM evidence_events WHERE id IN (
                SELECT id FROM evidence_events
                ORDER BY received_at DESC, id DESC
                LIMIT -1 OFFSET ?1
            )",
            [maximum as i64],
        )?)
    }

    pub fn healthcheck(&self) -> Result<()> {
        self.connection()?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn health(&self) -> Result<()> {
        self.healthcheck()
    }
}

fn column_exists(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info('evidence_events') WHERE name = ?1
        )",
        [name],
        |row| row.get(0),
    )
}

struct RawEvent {
    id: String,
    received_at: String,
    kind: String,
    token: Option<String>,
    route_id: Option<String>,
    source_ip: String,
    method: Option<String>,
    path: Option<String>,
    query: Option<String>,
    headers: String,
    body: Option<String>,
    dns_name: Option<String>,
    dns_type: Option<String>,
    dns_answer: Option<String>,
    dns_sequence: Option<u64>,
}

impl RawEvent {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            received_at: row.get(1)?,
            kind: row.get(2)?,
            token: row.get(3)?,
            route_id: row.get(4)?,
            source_ip: row.get(5)?,
            method: row.get(6)?,
            path: row.get(7)?,
            query: row.get(8)?,
            headers: row.get(9)?,
            body: row.get(10)?,
            dns_name: row.get(11)?,
            dns_type: row.get(12)?,
            dns_answer: row.get(13)?,
            dns_sequence: row.get(14)?,
        })
    }

    fn try_into_event(self) -> Result<EvidenceEvent> {
        let received_at = DateTime::parse_from_rfc3339(&self.received_at)
            .map_err(|source| StorageError::Timestamp {
                event_id: self.id.clone(),
                source,
            })?
            .with_timezone(&Utc);
        let headers = match serde_json::from_str::<BTreeMap<String, Vec<String>>>(&self.headers) {
            Ok(headers) => headers,
            Err(source) => serde_json::from_str::<BTreeMap<String, String>>(&self.headers)
                .map(|legacy| {
                    legacy
                        .into_iter()
                        .map(|(name, value)| (name, vec![value]))
                        .collect()
                })
                .map_err(|_| StorageError::Headers {
                    event_id: self.id.clone(),
                    source,
                })?,
        };
        Ok(EvidenceEvent {
            id: self.id,
            received_at,
            kind: self.kind,
            token: self.token,
            route_id: self.route_id,
            source_ip: self.source_ip,
            method: self.method,
            path: self.path,
            query: self.query,
            headers,
            body: self.body,
            dns_name: self.dns_name,
            dns_type: self.dns_type,
            dns_answer: self.dns_answer,
            dns_sequence: self.dns_sequence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, token: &str, received_at: DateTime<Utc>) -> EvidenceEvent {
        EvidenceEvent {
            id: id.into(),
            received_at,
            kind: "http".into(),
            token: Some(token.into()),
            route_id: Some("metadata".into()),
            source_ip: "192.0.2.1".into(),
            method: Some("POST".into()),
            path: Some(format!("/c/{token}")),
            query: Some("probe=yes".into()),
            headers: BTreeMap::from([("content-type".into(), vec!["text/plain".into()])]),
            body: Some("evidence".into()),
            dns_name: None,
            dns_type: None,
            dns_answer: None,
            dns_sequence: None,
        }
    }

    #[test]
    fn round_trips_and_filters_events() {
        let store = EvidenceStore::in_memory().unwrap();
        let first = event("one", "alpha", Utc::now() - chrono::Duration::seconds(1));
        let second = event("two", "beta", Utc::now());
        store.insert(&first).unwrap();
        store.insert(&second).unwrap();

        assert_eq!(store.count().unwrap(), 2);
        assert_eq!(
            store.get("one").unwrap().unwrap().body.as_deref(),
            Some("evidence")
        );
        let matches = store
            .list(&EventQuery {
                token: Some("beta".into()),
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "two");
        assert_eq!(
            store
                .count_filtered(&EventQuery {
                    search: Some("beta".into()),
                    ..Default::default()
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn reads_legacy_single_value_header_maps() {
        let store = EvidenceStore::in_memory().unwrap();
        store
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO evidence_events (
                    id, received_at, kind, source_ip, headers
                 ) VALUES (?1, ?2, 'http', '192.0.2.1', ?3)",
                params!["legacy", Utc::now().to_rfc3339(), r#"{"x-test":"one"}"#],
            )
            .unwrap();
        assert_eq!(
            store.get("legacy").unwrap().unwrap().headers["x-test"],
            ["one"]
        );
    }

    #[test]
    fn version_one_migration_recovers_a_preexisting_partial_column() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_metadata (version INTEGER NOT NULL);
                 INSERT INTO schema_metadata VALUES (1);
                 CREATE TABLE evidence_events (
                    id TEXT PRIMARY KEY NOT NULL, received_at TEXT NOT NULL,
                    kind TEXT NOT NULL, token TEXT, route_id TEXT, source_ip TEXT NOT NULL,
                    method TEXT, path TEXT, query TEXT, headers TEXT NOT NULL, body TEXT,
                    dns_name TEXT, dns_type TEXT, dns_answer TEXT
                 );",
            )
            .unwrap();
        let store = EvidenceStore::from_connection(connection).unwrap();
        let connection = store.connection().unwrap();
        assert!(column_exists(&connection, "dns_answer").unwrap());
        assert!(column_exists(&connection, "dns_sequence").unwrap());
        assert_eq!(
            connection
                .query_row("SELECT version FROM schema_metadata", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn prunes_by_age_and_count() {
        let store = EvidenceStore::in_memory().unwrap();
        let now = Utc::now();
        store
            .insert(&event("old", "a", now - chrono::Duration::days(3)))
            .unwrap();
        store
            .insert(&event("middle", "b", now - chrono::Duration::days(2)))
            .unwrap();
        store.insert(&event("new", "c", now)).unwrap();

        assert_eq!(
            store
                .prune_older_than(now - chrono::Duration::days(2))
                .unwrap(),
            1
        );
        assert_eq!(store.prune_to_limit(1).unwrap(), 1);
        assert_eq!(store.get("new").unwrap().unwrap().id, "new");
    }
}
