use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::Path;

use crate::models::{Article, Rule, Source};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                last_updated TEXT
            );

            CREATE TABLE IF NOT EXISTS articles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_id INTEGER NOT NULL REFERENCES sources(id),
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                content TEXT,
                published_at TEXT,
                word_count INTEGER NOT NULL DEFAULT 0,
                tags TEXT NOT NULL DEFAULT '[]',
                read INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                rule_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_articles_source ON articles(source_id);
            CREATE INDEX IF NOT EXISTS idx_articles_read ON articles(read);
            "#,
        )?;
        Ok(())
    }

    // === Sources ===

    pub fn add_source(&self, url: &str, title: &str, tags: &HashSet<String>) -> Result<i64> {
        let tags_json = serde_json::to_string(tags)?;
        self.conn.execute(
            "INSERT INTO sources (url, title, tags) VALUES (?1, ?2, ?3)",
            params![url, title, tags_json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_sources(&self) -> Result<Vec<Source>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, url, title, tags, last_updated FROM sources")?;
        let sources = stmt
            .query_map([], |row| {
                let tags_json: String = row.get(3)?;
                let last_updated: Option<String> = row.get(4)?;
                Ok(Source {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get(2)?,
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    last_updated: last_updated
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(sources)
    }

    pub fn get_source(&self, id: i64) -> Result<Option<Source>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, url, title, tags, last_updated FROM sources WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let tags_json: String = row.get(3)?;
            let last_updated: Option<String> = row.get(4)?;
            Ok(Some(Source {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                last_updated: last_updated
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn update_source_tags(&self, id: i64, tags: &HashSet<String>) -> Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        self.conn.execute(
            "UPDATE sources SET tags = ?1 WHERE id = ?2",
            params![tags_json, id],
        )?;
        Ok(())
    }

    pub fn update_source_timestamp(&self, id: i64, timestamp: DateTime<Utc>) -> Result<()> {
        self.conn.execute(
            "UPDATE sources SET last_updated = ?1 WHERE id = ?2",
            params![timestamp.to_rfc3339(), id],
        )?;
        Ok(())
    }

    // === Articles ===

    pub fn add_article(&self, article: &Article) -> Result<i64> {
        let tags_json = serde_json::to_string(&article.tags)?;
        let published = article.published_at.map(|d| d.to_rfc3339());
        self.conn.execute(
            r#"INSERT OR IGNORE INTO articles
               (source_id, url, title, content, published_at, word_count, tags, read)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                article.source_id,
                article.url,
                article.title,
                article.content,
                published,
                article.word_count,
                tags_json,
                article.read as i32,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn article_exists(&self, url: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM articles WHERE url = ?1",
            params![url],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn get_articles(&self) -> Result<Vec<Article>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, source_id, url, title, content, published_at, word_count, tags, read
               FROM articles ORDER BY published_at DESC"#,
        )?;
        let articles = stmt
            .query_map([], |row| {
                let tags_json: String = row.get(7)?;
                let published: Option<String> = row.get(5)?;
                let read_int: i32 = row.get(8)?;
                Ok(Article {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    url: row.get(2)?,
                    title: row.get(3)?,
                    content: row.get(4)?,
                    published_at: published
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                    word_count: row.get(6)?,
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    read: read_int != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(articles)
    }

    pub fn update_article_tags(&self, id: i64, tags: &HashSet<String>) -> Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        self.conn.execute(
            "UPDATE articles SET tags = ?1 WHERE id = ?2",
            params![tags_json, id],
        )?;
        Ok(())
    }

    pub fn mark_read(&self, id: i64, read: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE articles SET read = ?1 WHERE id = ?2",
            params![read as i32, id],
        )?;
        Ok(())
    }

    // === Rules ===

    pub fn add_rule(&self, rule: &Rule) -> Result<i64> {
        let json = serde_json::to_string(rule)?;
        self.conn
            .execute("INSERT INTO rules (rule_json) VALUES (?1)", params![json])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_rules(&self) -> Result<Vec<(i64, Rule)>> {
        let mut stmt = self.conn.prepare("SELECT id, rule_json FROM rules")?;
        let rules = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let json: String = row.get(1)?;
                let rule: Rule = serde_json::from_str(&json).unwrap();
                Ok((id, rule))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rules)
    }

    pub fn delete_rule(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM rules WHERE id = ?1", params![id])?;
        Ok(())
    }
}
