use anyhow::Result;
use feed_rs::parser;
use reqwest::Client;
use chrono::{DateTime, Utc};
use crate::db::Database;

pub struct FeedManager {
    db: Database,
    client: Client,
}

impl FeedManager {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            client: Client::new(),
        }
    }

    pub async fn get_feeds(&self) -> Result<Vec<(i64, String, Option<String>)>> {
        self.db.get_feeds().await
    }

    pub async fn add_feed(&self, url: &str) -> Result<()> {
        // Fetch and parse the feed
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        let feed = parser::parse(&bytes[..])?;

        // Add feed to database
        let feed_id = self.db.add_feed(url, Some(&feed.title.unwrap().content)).await?;

        // Add feed items
        for entry in feed.entries {
            let published_at = entry.published.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc));
            
            self.db.add_item(
                feed_id,
                &entry.title.content,
                &entry.links[0].href,
                entry.content.map(|c| c.body),
                published_at,
            ).await?;
        }

        self.db.update_feed_timestamp(feed_id).await?;
        Ok(())
    }

    pub async fn update_feeds(&self) -> Result<()> {
        let feeds = self.db.get_feeds().await?;
        
        for (feed_id, url, _) in feeds {
            // Fetch and parse the feed
            let response = self.client.get(&url).send().await?;
            let bytes = response.bytes().await?;
            let feed = parser::parse(&bytes[..])?;

            // Update feed items
            for entry in feed.entries {
                let published_at = entry.published.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc));
                
                self.db.add_item(
                    feed_id,
                    &entry.title.content,
                    &entry.links[0].href,
                    entry.content.map(|c| c.body),
                    published_at,
                ).await?;
            }

            self.db.update_feed_timestamp(feed_id).await?;
        }

        Ok(())
    }
} 
