use sqlx::{sqlite::SqlitePool, Row};
use anyhow::Result;
use chrono::{DateTime, Utc};

pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn new() -> Result<Self> {
        let pool = SqlitePool::connect("sqlite:tagrss.db").await?;
        let db = Self { pool };
        db.init().await?;
        Ok(db)
    }

    pub async fn init(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS feeds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL UNIQUE,
                title TEXT,
                last_updated DATETIME,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                feed_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                url TEXT NOT NULL,
                content TEXT,
                published_at DATETIME,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (feed_id) REFERENCES feeds(id),
                UNIQUE(feed_id, url)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn add_feed(&self, url: &str, title: Option<&str>) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO feeds (url, title)
            VALUES (?, ?)
            RETURNING id
            "#,
        )
        .bind(url)
        .bind(title)
        .fetch_one(&self.pool)
        .await?;
    
        Ok(result.get(0))
    }
    
    pub async fn get_feeds(&self) -> Result<Vec<(i64, String, Option<String>)>> {
        let feeds = sqlx::query(
            r#"
            SELECT id, url, title FROM feeds
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
    
        Ok(feeds
            .into_iter()
            .map(|row| (row.get(0), row.get(1), row.get(2)))
            .collect())
    }
    
    pub async fn add_item(
        &self,
        feed_id: i64,
        title: &str,
        url: &str,
        content: Option<&str>,
        published_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO items (feed_id, title, url, content, published_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(feed_id)
        .bind(title)
        .bind(url)
        .bind(content)
        .bind(published_at)
        .execute(&self.pool)
        .await?;
    
        Ok(())
    }
    
    pub async fn update_feed_timestamp(&self, feed_id: i64) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE feeds
            SET last_updated = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(feed_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
} 
