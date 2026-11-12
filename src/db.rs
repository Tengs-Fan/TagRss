//! The database: a disposable cache of what feeds said, plus read marks.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Bumped whenever the schema changes; a database with another version is dropped and recreated.
const SCHEMA_VERSION: i32 = 3;

pub struct Database {
    conn: Connection,
}

/// An item as a feed reported it, before it has an id.
pub struct NewObservation<'a> {
    pub source_url: &'a str,
    pub item_key: &'a str,
    pub url: Option<&'a str>,
    pub title: &'a str,
    pub published_at: Option<DateTime<Utc>>,
    pub word_count: Option<u32>,
    /// HTML shown in the preview pane.
    pub content: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Mark {
    pub read: bool,
    pub marked_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub id: i64,
    pub source_url: String,
    pub url: Option<String>,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    pub first_seen_at: DateTime<Utc>,
    pub word_count: Option<u32>,
    pub mark: Option<Mark>,
}

#[derive(Debug, Clone)]
pub struct Fetch {
    pub feed_title: Option<String>,
    pub fetched_at: DateTime<Utc>,
    pub error: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS observations (
    id            INTEGER PRIMARY KEY,
    source_url    TEXT NOT NULL,
    item_key      TEXT NOT NULL,
    url           TEXT,
    title         TEXT NOT NULL,
    published_at  TEXT,
    first_seen_at TEXT NOT NULL,
    word_count    INTEGER,
    content       TEXT,
    UNIQUE (source_url, item_key)
);
CREATE TABLE IF NOT EXISTS marks (
    observation_id INTEGER PRIMARY KEY REFERENCES observations(id),
    read           INTEGER NOT NULL,
    marked_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS fetches (
    source_url TEXT PRIMARY KEY,
    feed_title TEXT,
    fetched_at TEXT NOT NULL,
    error      TEXT
);
"#;

fn to_text(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

fn from_text(s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

impl Database {
    /// Opens the database, creating it and its directory if missing.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("cannot open database {}", path.display()))?;
        // Sync writes on its own connection while a user interface reads and marks.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 2 {
            // Version 3 only adds the preview content; keep the observations and read marks.
            conn.execute_batch("ALTER TABLE observations ADD COLUMN content TEXT")?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version != SCHEMA_VERSION {
            // The database is a cache: a schema change costs the same as losing it.
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for table in tables {
                conn.execute_batch(&format!("DROP TABLE \"{table}\""))?;
            }
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Stores an observation the first time it is seen; returns false if it was already stored.
    pub fn insert_observation(&self, o: &NewObservation, now: DateTime<Utc>) -> Result<bool> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO observations
                 (source_url, item_key, url, title, published_at, first_seen_at, word_count, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                o.source_url,
                o.item_key,
                o.url,
                o.title,
                o.published_at.map(to_text),
                to_text(now),
                o.word_count,
                o.content,
            ],
        )?;
        if inserted == 0 && o.content.is_some() {
            // Observations stored before content was kept get it on the next sync.
            self.conn.execute(
                "UPDATE observations SET content = ?1 WHERE source_url = ?2 AND item_key = ?3 AND content IS NULL",
                params![o.content, o.source_url, o.item_key],
            )?;
        }
        Ok(inserted == 1)
    }

    pub fn record_fetch(&self, source_url: &str, feed_title: Option<&str>, now: DateTime<Utc>, error: Option<&str>) -> Result<()> {
        // A failed fetch keeps the title from the last successful one.
        self.conn.execute(
            "INSERT INTO fetches (source_url, feed_title, fetched_at, error) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (source_url) DO UPDATE SET
                 feed_title = COALESCE(excluded.feed_title, feed_title),
                 fetched_at = excluded.fetched_at,
                 error = excluded.error",
            params![source_url, feed_title, to_text(now), error],
        )?;
        Ok(())
    }

    pub fn fetches(&self) -> Result<HashMap<String, Fetch>> {
        let mut stmt = self.conn.prepare("SELECT source_url, feed_title, fetched_at, error FROM fetches")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Fetch {
                    feed_title: r.get(1)?,
                    fetched_at: from_text(&r.get::<_, String>(2)?)?,
                    error: r.get(3)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn observations(&self) -> Result<Vec<Observation>> {
        let mut stmt = self.conn.prepare(
            "SELECT o.id, o.source_url, o.url, o.title, o.published_at, o.first_seen_at, o.word_count,
                    m.read, m.marked_at
             FROM observations o LEFT JOIN marks m ON m.observation_id = o.id",
        )?;
        let rows = stmt.query_map([], |r| {
            let mark = match r.get::<_, Option<bool>>(7)? {
                Some(read) => Some(Mark { read, marked_at: from_text(&r.get::<_, String>(8)?)? }),
                None => None,
            };
            Ok(Observation {
                id: r.get(0)?,
                source_url: r.get(1)?,
                url: r.get(2)?,
                title: r.get(3)?,
                published_at: r.get::<_, Option<String>>(4)?.map(|s| from_text(&s)).transpose()?,
                first_seen_at: from_text(&r.get::<_, String>(5)?)?,
                word_count: r.get(6)?,
                mark,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The longest stored content among the given observations, for the preview pane.
    pub fn content(&self, observation_ids: &[i64]) -> Result<Option<String>> {
        let mut best: Option<String> = None;
        let mut stmt = self.conn.prepare("SELECT content FROM observations WHERE id = ?1")?;
        for id in observation_ids {
            let content: Option<String> = stmt.query_row(params![id], |r| r.get(0))?;
            if let Some(c) = content {
                if best.as_ref().is_none_or(|b| c.len() > b.len()) {
                    best = Some(c);
                }
            }
        }
        Ok(best)
    }

    /// Writes the same mark onto every given observation, in one transaction.
    pub fn set_marks(&mut self, observation_ids: &[i64], read: bool, now: DateTime<Utc>) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO marks (observation_id, read, marked_at) VALUES (?1, ?2, ?3)",
            )?;
            for id in observation_ids {
                stmt.execute(params![id, read, to_text(now)])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
    }

    fn obs<'a>(source: &'a str, key: &'a str) -> NewObservation<'a> {
        NewObservation {
            source_url: source,
            item_key: key,
            url: Some("https://example.com/a"),
            title: "A",
            published_at: Some(now() - Duration::hours(1)),
            word_count: None,
            content: None,
        }
    }

    #[test]
    fn content_is_backfilled_once_and_the_longest_wins() {
        let db = Database::open_in_memory().unwrap();
        db.insert_observation(&obs("s1", "k"), now()).unwrap();
        let mut with = obs("s1", "k");
        with.content = Some("<p>short</p>");
        assert!(!db.insert_observation(&with, now()).unwrap());
        let mut other = obs("s2", "k");
        other.content = Some("<p>the longer text</p>");
        db.insert_observation(&other, now()).unwrap();
        let ids: Vec<i64> = db.observations().unwrap().iter().map(|o| o.id).collect();
        assert_eq!(db.content(&ids).unwrap().as_deref(), Some("<p>the longer text</p>"));
        assert_eq!(db.content(&ids[..1]).unwrap().as_deref(), Some("<p>short</p>"));
    }

    #[test]
    fn observations_are_unique_per_source_and_item() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.insert_observation(&obs("s1", "k"), now()).unwrap());
        assert!(!db.insert_observation(&obs("s1", "k"), now()).unwrap());
        // The same item from a second feed is a second observation.
        assert!(db.insert_observation(&obs("s2", "k"), now()).unwrap());
        assert_eq!(db.observations().unwrap().len(), 2);
    }

    #[test]
    fn marks_round_trip_with_their_time() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_observation(&obs("s", "k"), now()).unwrap();
        let id = db.observations().unwrap()[0].id;
        assert!(db.observations().unwrap()[0].mark.is_none());
        db.set_marks(&[id], true, now()).unwrap();
        let mark = db.observations().unwrap()[0].mark.clone().unwrap();
        assert!(mark.read && mark.marked_at == now());
        db.set_marks(&[id], false, now() + Duration::minutes(1)).unwrap();
        assert!(!db.observations().unwrap()[0].mark.clone().unwrap().read);
    }

    #[test]
    fn failed_fetch_keeps_the_last_title() {
        let db = Database::open_in_memory().unwrap();
        db.record_fetch("s", Some("Title"), now(), None).unwrap();
        db.record_fetch("s", None, now(), Some("timeout")).unwrap();
        let f = &db.fetches().unwrap()["s"];
        assert_eq!(f.feed_title.as_deref(), Some("Title"));
        assert_eq!(f.error.as_deref(), Some("timeout"));
    }

    #[test]
    fn an_old_schema_is_dropped_and_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/tagrss.db");
        {
            let conn = Connection::open(&path).unwrap_or_else(|_| {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                Connection::open(&path).unwrap()
            });
            conn.execute_batch("CREATE TABLE entries (id INTEGER); CREATE TABLE marks (entry_id INTEGER, read INTEGER);")
                .unwrap();
        }
        let db = Database::open(&path).unwrap();
        db.insert_observation(&obs("s", "k"), now()).unwrap();
        assert_eq!(db.observations().unwrap().len(), 1);
        drop(db);
        // Reopening at the current version keeps the data.
        assert_eq!(Database::open(&path).unwrap().observations().unwrap().len(), 1);
    }
}
