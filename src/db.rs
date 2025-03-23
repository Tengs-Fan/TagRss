use sqlx::{sqlite::SqlitePool, Row};
use anyhow::Result;
use chrono::{DateTime, Utc};
use crate::tag::Tag;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(url).await?;
        let db = Self { pool };
        db.init().await?;
        Ok(db)
    }

    async fn init(&self) -> Result<()> {
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

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS item_tags (
                item_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                PRIMARY KEY (item_id, tag_id),
                FOREIGN KEY (item_id) REFERENCES items(id) ON DELETE CASCADE,
                FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
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
        content: Option<String>,
        published_at: Option<DateTime<Utc>>,
    ) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT OR REPLACE INTO items (feed_id, title, url, content, published_at)
            VALUES (?, ?, ?, ?, ?)
            RETURNING id
            "#,
        )
        .bind(feed_id)
        .bind(title)
        .bind(url)
        .bind(content)
        .bind(published_at)
        .fetch_one(&self.pool)
        .await?;
    
        Ok(result.get(0))
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

    pub async fn add_tag(&self, name: &str) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO tags (name)
            VALUES (?)
            RETURNING id
            "#,
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await?;
    
        Ok(result.get(0))
    }
    
    pub async fn get_tag_by_name(&self, name: &str) -> Result<Option<Tag>> {
        let result = sqlx::query(
            r#"
            SELECT id, name FROM tags
            WHERE name = ?
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        
        if let Some(row) = result {
            Ok(Some(Tag {
                id: row.get(0),
                name: row.get(1),
            }))
        } else {
            Ok(None)
        }
    }
    
    pub async fn get_tag_by_id(&self, id: i64) -> Result<Option<Tag>> {
        let result = sqlx::query(
            r#"
            SELECT id, name FROM tags
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        
        if let Some(row) = result {
            Ok(Some(Tag {
                id: row.get(0),
                name: row.get(1),
            }))
        } else {
            Ok(None)
        }
    }
    
    pub async fn get_all_tags(&self) -> Result<Vec<Tag>> {
        let tags = sqlx::query(
            r#"
            SELECT id, name FROM tags
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        
        Ok(tags
            .into_iter()
            .map(|row| Tag {
                id: row.get(0),
                name: row.get(1),
            })
            .collect())
    }
    
    pub async fn add_tag_to_item(&self, item_id: i64, tag_id: i64) -> Result<()> {
        let existing = sqlx::query(
            r#"
            SELECT 1 FROM item_tags
            WHERE item_id = ? AND tag_id = ?
            "#,
        )
        .bind(item_id)
        .bind(tag_id)
        .fetch_optional(&self.pool)
        .await?;
        
        if existing.is_none() {
            sqlx::query(
                r#"
                INSERT INTO item_tags (item_id, tag_id)
                VALUES (?, ?)
                "#,
            )
            .bind(item_id)
            .bind(tag_id)
            .execute(&self.pool)
            .await?;
        }
        
        Ok(())
    }
    
    pub async fn get_item_tags(&self, item_id: i64) -> Result<Vec<Tag>> {
        let tags = sqlx::query(
            r#"
            SELECT t.id, t.name
            FROM tags t
            JOIN item_tags it ON t.id = it.tag_id
            WHERE it.item_id = ?
            ORDER BY t.name
            "#,
        )
        .bind(item_id)
        .fetch_all(&self.pool)
        .await?;
        
        Ok(tags
            .into_iter()
            .map(|row| Tag {
                id: row.get(0),
                name: row.get(1),
            })
            .collect())
    }
    
    pub async fn get_items_by_tag(&self, tag_id: i64) -> Result<Vec<i64>> {
        let items = sqlx::query(
            r#"
            SELECT item_id
            FROM item_tags
            WHERE tag_id = ?
            "#,
        )
        .bind(tag_id)
        .fetch_all(&self.pool)
        .await?;
        
        Ok(items
            .into_iter()
            .map(|row| row.get(0))
            .collect())
    }

    pub async fn get_all_items(&self) -> Result<Vec<crate::models::FeedItem>> {
        let items = sqlx::query(
            r#"
            SELECT i.id, i.title, i.url, i.content, i.published_at, i.created_at
            FROM items i
            ORDER BY i.created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        
        let mut result = Vec::new();
        
        for row in items {
            // For each item, get its tags
            let item_id: i64 = row.get(0);
            let tags = self.get_item_tags(item_id).await?;
            let tag_ids = tags.into_iter().map(|tag| tag.id).collect();
            
            result.push(crate::models::FeedItem {
                id: item_id,
                tags: tag_ids,
                title: row.get(1),
                url: row.get(2),
                content: row.get(3),
                published_at: row.get(4),
                created_at: row.get(5),
            });
        }
        
        Ok(result)
    }
} 
