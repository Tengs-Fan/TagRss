use anyhow::{Ok, Result};
use clap::builder::Str;
use feed_rs::parser;
use reqwest::Client;
use crate::db::Database;
use crate::models::FeedItem;
use crate::tag::TagManager;

pub struct FeedManager {
    pub db: Database,
    client: Client,
}

impl FeedManager {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            client: Client::new(),
        }
    }

    pub async fn add_feed(&self, url: &str) -> Result<()> {
        // Fetch and parse the feed
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        let feed = parser::parse(&bytes[..])?;

        // Add feed to database
        let title = feed.title.as_ref().map(|t| t.content.clone());
        let feed_id = self.db.add_feed(url, title.as_deref()).await?;

        self.update_feed(feed_id, &url, None).await?;

        Ok(())
    }
    
    pub async fn update_feeds(&self, tag_manager: Option<&TagManager>) -> Result<()> {
        let feeds = self.db.get_feeds().await?;
        
        for (feed_id, url, _) in feeds {
            self.update_feed(feed_id, &url, tag_manager).await?;
        }

        Ok(())
    }

    pub async fn update_feed(&self, feed_id: i64, url: &str, tag_manager: Option<&TagManager>) -> Result<()> {
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        let feed = parser::parse(&bytes[..])?;

        // Update feed items
        for entry in feed.entries {
            let title = entry.title.as_ref().map(|t| t.content.clone()).unwrap_or_else(|| "Untitled".to_string());
            let link = entry.links.first().map(|l| l.href.clone()).unwrap_or_else(|| "".to_string());
            let content = entry.content.as_ref().and_then(|c| c.body.clone());
            
            let item_id = self.db.add_item(
                feed_id,
                &title,
                &link,
                content,
                entry.published,
            ).await?;
            
            // Apply tag rules if a tag manager is provided
            if let Some(tag_manager) = tag_manager {
                let mut feed_item = FeedItem {
                    id: item_id,
                    tags: Vec::new(),
                    title,
                    url: link,
                    content,
                    published_at: entry.published,
                    created_at: chrono::Utc::now(),
                };
                
                tag_manager.apply_rules(&mut feed_item)?;
                
                // Update the database with the assigned tags
                for tag_id in feed_item.tags {
                    self.db.add_tag_to_item(item_id, tag_id).await?;
                }
            }
        }

        self.db.update_feed_timestamp(feed_id).await?;
        Ok(())
    }
    
    pub async fn apply_rules_to_existing_items(&self, tag_manager: &TagManager) -> Result<()> {
        // Get all items from the database
        // For each item, apply the rules and update the database
        let items = self.db.get_all_items().await?;
        
        for item in items {
            let mut feed_item = FeedItem {
                id: item.id,
                tags: Vec::new(), // We'll get the existing tags from the database
                title: item.title,
                url: item.url,
                content: item.content,
                published_at: item.published_at,
                created_at: item.created_at,
            };
            
            // Get existing tags for this item
            let existing_tags = self.db.get_item_tags(item.id).await?;
            feed_item.tags = existing_tags.into_iter().map(|tag| tag.id).collect();
            
            // Apply tag rules
            tag_manager.apply_rules(&mut feed_item)?;
            
            // Update the database with any new tags
            for tag_id in feed_item.tags {
                self.db.add_tag_to_item(item.id, tag_id).await?;
            }
        }
        
        Ok(())
    }
} 
