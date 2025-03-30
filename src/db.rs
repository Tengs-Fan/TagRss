use sqlx::{sqlite::SqlitePool, Row};
use anyhow::Result;
use log::{info, debug, error, warn};
use crate::models::FeedItem;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(url: &str) -> Result<Self> {
        debug!("Connecting to database at: {}", url);
        let pool = SqlitePool::connect(url).await?;
        let db = Self { pool };
        debug!("Database connection established");
        db.init().await?;
        info!("Database initialized successfully");
        Ok(db)
    }

    async fn init(&self) -> Result<()> {
        debug!("Creating feeds table if it doesn't exist");
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

        debug!("Creating items table if it doesn't exist");
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                feed_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                tags TEXT,
                url TEXT NOT NULL UNIQUE,
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

        debug!("Database schema initialized");
        Ok(())
    }

    pub async fn add_feed(&self, url: &str, title: Option<&str>) -> Result<i64> {
        debug!("Adding feed to database: {} ({})", url, title.unwrap_or("Untitled"));
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
    
        let id: i64 = result.get(0);
        debug!("Feed added with ID: {}", id);
        Ok(id)
    }
    
    pub async fn get_feeds(&self) -> Result<Vec<(i64, String, Option<String>)>> {
        debug!("Retrieving all feeds from database");
        let feeds = sqlx::query(
            r#"
            SELECT id, url, title FROM feeds
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
    
        let feed_count = feeds.len();
        debug!("Retrieved {} feeds from database", feed_count);
        Ok(feeds
            .into_iter()
            .map(|row| (row.get(0), row.get(1), row.get(2)))
            .collect())
    }

    pub async fn check_item_exists(&self, url: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            SELECT COUNT(*) FROM items WHERE url = ?
            "#,
        )
        .bind(url)
        .fetch_one(&self.pool)
        .await?;
    
        let count: i64 = result.get(0);
        Ok(count > 0)
    }
    
    pub async fn add_item(
        &self,
        feed: FeedItem,
    ) -> Result<()> {
        debug!("Adding/updating item: {}", feed.title);
        let tags_str = feed.tags.iter().map(|t| t.to_string()).collect::<Vec<String>>().join(",");
        
        let _ = sqlx::query(
            r#"
            INSERT OR REPLACE INTO items (feed_id, title, tags, url, content, published_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(feed.feed_id)
        .bind(feed.title)
        .bind(tags_str)
        .bind(feed.url)
        .bind(feed.content)
        .bind(feed.published_at)
        .execute(&self.pool)
        .await?;
    
        debug!("Item added/updated successfully");
        Ok(())
    }
    
    pub async fn update_feed_timestamp(&self, feed_id: i64) -> Result<()> {
        debug!("Updating last_updated timestamp for feed ID: {}", feed_id);
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

    pub async fn get_all_items(&self) -> Result<Vec<crate::models::FeedItem>> {
        debug!("Retrieving all items from database");
        let items = sqlx::query(
            r#"
            SELECT i.title, i.tags, i.url, i.content, i.published_at, i.feed_id
            FROM items i
            ORDER BY i.created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        
        let item_count = items.len();
        debug!("Retrieved {} items from database", item_count);
        
        let mut result = Vec::new();
        
        for row in items {
            result.push(FeedItem {
                feed_id: row.get(5),
                title: row.get(0),
                tags: {
                    let tags_str: String = row.get(1);
                    tags_str.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
                },
                url: row.get(2),
                content: row.get(3),
                published_at: row.get(4),
            });
        }
        
        Ok(result)
    }
} 
