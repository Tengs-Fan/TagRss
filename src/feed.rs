use anyhow::{Ok, Result};
use feed_rs::parser;
use reqwest::Client;
use crate::db::Database;
use crate::models::FeedItem;
use crate::tag::TagManager;
use std::collections::HashSet;

pub struct FeedManager {
    pub db: Database,
    client: Client,
    pub tag_manager: TagManager,
}

impl FeedManager {
    pub fn new(db: Database, tag_manager: TagManager) -> Self {
        Self {
            db,
            client: Client::new(),
            tag_manager,
        }
    }

    pub async fn add_feed(&self, url: &str) -> Result<()> {
        // Fetch and parse the feed
        let response = self.client.get(url).send().await?.bytes().await?;
        let feed = parser::parse(&response[..])?;

        // Add feed to database
        let title = feed.title.as_ref().map(|t| t.content.clone());
        let feed_id = self.db.add_feed(url, title.as_deref()).await?;

        self.update_feed(feed_id, &url).await?;

        Ok(())
    }
    
    pub async fn update_feeds(&self) -> Result<()> {
        let feeds = self.db.get_feeds().await?;
        
        for (feed_id, url, _) in feeds {
            self.update_feed(feed_id, &url).await?;
        }

        Ok(())
    }

    pub async fn update_feed(&self, feed_id: i64, url: &str) -> Result<()> {
        // Fetch the feed content
        let content = self.client.get(url).send().await?.bytes().await?;
        let feed = parser::parse(&content[..])?;
        
        // Process each entry
        for entry in feed.entries {
            let link = entry.links.first().map(|l| l.href.clone()).unwrap_or_else(|| "".to_string());
            // If the item already exists, skip it
            if !self.db.check_item_exists(&link).await? {
                return Ok(());
            }
            let title = entry.title.as_ref().map(|t| t.content.clone()).unwrap_or_else(|| "Untitled".to_string());
            let content = entry.content.as_ref().and_then(|c| c.body.clone());
            
            // Create feed item and apply rules
            let mut feed_item = FeedItem {
                feed_id,
                tags: HashSet::new(), // Use HashSet as defined in models
                title,
                url: link,
                content,
                published_at: entry.published,
            };
            
            // Apply tag rules
            self.tag_manager.apply_rules(&mut feed_item)?;

            // Add to database
            self.db.add_item(feed_item).await?;
        }

        self.db.update_feed_timestamp(feed_id).await?;
        Ok(())
    }
    
    pub async fn apply_rules_to_existing_items(&self) -> Result<()> {
        // Get all items from the database
        let items = self.db.get_all_items().await?;
        
        for mut item in items {
            // Apply tag rules directly to the items from database
            self.tag_manager.apply_rules(&mut item)?;
        }
        
        Ok(())
    }
} 
