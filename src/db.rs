//! The database: a disposable cache of what feeds said, plus read marks.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

pub struct Database {
    conn: Connection,
}

/// An entry as fetched, before it has an id.
pub struct NewEntry<'a> {
    pub source_url: &'a str,
    pub entry_key: &'a str,
    pub url: Option<&'a str>,
    pub title: &'a str,
    pub published_at: Option<DateTime<Utc>>,
    pub word_count: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub source_url: String,
    pub url: Option<String>,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    pub first_seen_at: DateTime<Utc>,
    pub word_count: Option<u32>,
    /// An explicit read (true) or unread (false) mark; None if never marked.
    pub mark: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Fetch {
    pub feed_title: Option<String>,
    pub fetched_at: DateTime<Utc>,
    pub error: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS entries (
    id            INTEGER PRIMARY KEY,
    source_url    TEXT NOT NULL,
    entry_key     TEXT NOT NULL,
    url           TEXT,
    title         TEXT NOT NULL,
    published_at  TEXT,
    first_seen_at TEXT NOT NULL,
    word_count    INTEGER,
    UNIQUE (source_url, entry_key)
);
CREATE TABLE IF NOT EXISTS marks (
    entry_id  INTEGER PRIMARY KEY REFERENCES entries(id),
    read      INTEGER NOT NULL,
    marked_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS fetches (
    source_url TEXT PRIMARY KEY,
    feed_title TEXT,
    fetched_at TEXT NOT NULL,
    error      TEXT
);
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
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
    /// A newly created database records `now` as `rebuilt_at`, so it starts at inbox zero.
    pub fn open(path: &Path, now: DateTime<Utc>) -> Result<Self> {
        let created = !path.exists();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("cannot open database {}", path.display()))?;
        Self::init(conn, created.then_some(now))
    }

    /// An in-memory database for tests; `rebuilt_at` is whatever the caller passes.
    pub fn open_in_memory(rebuilt_at: Option<DateTime<Utc>>) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, rebuilt_at)
    }

    fn init(conn: Connection, rebuilt_at: Option<DateTime<Utc>>) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        if let Some(t) = rebuilt_at {
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('rebuilt_at', ?1)",
                params![to_text(t)],
            )?;
        }
        Ok(Self { conn })
    }

    pub fn rebuilt_at(&self) -> Result<Option<DateTime<Utc>>> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'rebuilt_at'", [], |r| r.get(0))
            .optional()?;
        Ok(value.map(|v| from_text(&v)).transpose()?)
    }

    /// Stores an entry the first time it is seen; returns false if it was already stored.
    pub fn insert_entry(&self, e: &NewEntry, now: DateTime<Utc>) -> Result<bool> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO entries
                 (source_url, entry_key, url, title, published_at, first_seen_at, word_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                e.source_url,
                e.entry_key,
                e.url,
                e.title,
                e.published_at.map(to_text),
                to_text(now),
                e.word_count,
            ],
        )?;
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

    fn entries_where(&self, clause: &str, args: impl rusqlite::Params) -> Result<Vec<Entry>> {
        let sql = format!(
            "SELECT e.id, e.source_url, e.url, e.title, e.published_at, e.first_seen_at, e.word_count, m.read
             FROM entries e LEFT JOIN marks m ON m.entry_id = e.id {clause}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(args, |r| {
            Ok(Entry {
                id: r.get(0)?,
                source_url: r.get(1)?,
                url: r.get(2)?,
                title: r.get(3)?,
                published_at: r.get::<_, Option<String>>(4)?.map(|s| from_text(&s)).transpose()?,
                first_seen_at: from_text(&r.get::<_, String>(5)?)?,
                word_count: r.get(6)?,
                mark: r.get::<_, Option<bool>>(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        self.entries_where("", [])
    }

    pub fn entry(&self, id: i64) -> Result<Option<Entry>> {
        Ok(self.entries_where("WHERE e.id = ?1", params![id])?.into_iter().next())
    }

    /// Marks an entry read or unread; returns false if there is no such entry.
    pub fn set_mark(&self, id: i64, read: bool, now: DateTime<Utc>) -> Result<bool> {
        if self.entry(id)?.is_none() {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO marks (entry_id, read, marked_at) VALUES (?1, ?2, ?3)",
            params![id, read, to_text(now)],
        )?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
    }

    fn entry<'a>(source: &'a str, key: &'a str) -> NewEntry<'a> {
        NewEntry {
            source_url: source,
            entry_key: key,
            url: Some("https://example.com/a"),
            title: "A",
            published_at: Some(now() - Duration::hours(1)),
            word_count: None,
        }
    }

    #[test]
    fn entries_are_unique_per_source_not_per_url() {
        let db = Database::open_in_memory(None).unwrap();
        assert!(db.insert_entry(&entry("s1", "k"), now()).unwrap());
        assert!(!db.insert_entry(&entry("s1", "k"), now()).unwrap());
        // Same entry, same URL, a second feed: kept.
        assert!(db.insert_entry(&entry("s2", "k"), now()).unwrap());
        assert_eq!(db.entries().unwrap().len(), 2);
    }

    #[test]
    fn marks_round_trip() {
        let db = Database::open_in_memory(None).unwrap();
        db.insert_entry(&entry("s", "k"), now()).unwrap();
        let id = db.entries().unwrap()[0].id;
        assert_eq!(db.entry(id).unwrap().unwrap().mark, None);
        assert!(db.set_mark(id, true, now()).unwrap());
        assert_eq!(db.entry(id).unwrap().unwrap().mark, Some(true));
        assert!(db.set_mark(id, false, now()).unwrap());
        assert_eq!(db.entry(id).unwrap().unwrap().mark, Some(false));
        assert!(!db.set_mark(9999, true, now()).unwrap());
    }

    #[test]
    fn failed_fetch_keeps_the_last_title() {
        let db = Database::open_in_memory(None).unwrap();
        db.record_fetch("s", Some("Title"), now(), None).unwrap();
        db.record_fetch("s", None, now(), Some("timeout")).unwrap();
        let f = &db.fetches().unwrap()["s"];
        assert_eq!(f.feed_title.as_deref(), Some("Title"));
        assert_eq!(f.error.as_deref(), Some("timeout"));
    }

    #[test]
    fn a_new_database_file_records_rebuilt_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/tagrss.db");
        let db = Database::open(&path, now()).unwrap();
        assert_eq!(db.rebuilt_at().unwrap(), Some(now()));
        drop(db);
        let reopened = Database::open(&path, now() + Duration::days(1)).unwrap();
        assert_eq!(reopened.rebuilt_at().unwrap(), Some(now()));
    }
}
