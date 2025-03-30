use anyhow::{Ok, Result};
use feed_rs::parser;
use reqwest::Client;
use log::{info, debug, error, warn};
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
        debug!("Initializing FeedManager");
        Self {
            db,
            client: Client::new(),
            tag_manager,
        }
    }

    pub async fn add_feed(&self, url: &str) -> Result<()> {
        debug!("Fetching feed from URL: {}", url);
        // Fetch and parse the feed
        let response = match self.client.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to fetch feed from {}: {}", url, e);
                return Err(e.into());
            }
        };
        
        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to get response body from {}: {}", url, e);
                return Err(e.into());
            }
        };
        
        let feed = parser::parse(&bytes[..]);
        let feed = match feed {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to parse feed from {}: {}", url, e);
                return Err(e.into());
            }
        };

        // Add feed to database
        let title = feed.title.as_ref().map(|t| t.content.clone());
        debug!("Adding feed to database: {} ({})", title.as_deref().unwrap_or("Untitled"), url);
        let feed_id = self.db.add_feed(url, title.as_deref()).await?;
        info!("Added feed: {} with ID: {}", url, feed_id);

        self.update_feed(feed_id, &url).await?;

        Ok(())
    }
    
    pub async fn update_feeds(&self) -> Result<()> {
        debug!("Starting update of all feeds");
        let feeds = self.db.get_feeds().await?;
        
        info!("Updating {} feeds", feeds.len());
        for (feed_id, url, title) in feeds {
            let feed_name = title.unwrap_or_else(|| url.clone());
            debug!("Updating feed: {} (ID: {})", feed_name, feed_id);
            if let Err(e) = self.update_feed(feed_id, &url).await {
                error!("Failed to update feed {}: {}", feed_name, e);
            }
        }

        info!("Completed feed updates");
        Ok(())
    }

    pub async fn update_feed(&self, feed_id: i64, url: &str) -> Result<()> {
        debug!("Fetching content for feed ID {}: {}", feed_id, url);
        // Fetch the feed content
        let content = self.client.get(url).send().await?.bytes().await?;
        let feed = parser::parse(&content[..])?;
        
        let mut new_items = 0;
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
                title: title.clone(),
                url: link.clone(),
                content,
                published_at: entry.published,
            };
            
            // Apply tag rules
            debug!("Applying tag rules to item: {}", title);
            self.tag_manager.apply_rules(&mut feed_item)?;

            // Add to database
            debug!("Adding item to database: {}", title);
            self.db.add_item(feed_item).await?;
            new_items += 1;
        }

        debug!("Added {} new items for feed ID: {}", new_items, feed_id);
        self.db.update_feed_timestamp(feed_id).await?;
        Ok(())
    }
    
    pub async fn apply_rules_to_existing_items(&self) -> Result<()> {
        info!("Applying tag rules to all existing items");
        // Get all items from the database
        let items = self.db.get_all_items().await?;
        debug!("Found {} items to process", items.len());
        
        let mut updated_items = 0;
        for mut item in items {
            // Apply tag rules directly to the items from database
            let initial_tags = item.tags.len();
            self.tag_manager.apply_rules(&mut item)?;
            
            if item.tags.len() > initial_tags {
                debug!("Updated tags for item: {}", item.title);
                updated_items += 1;
            }
        }
        
        info!("Updated tags for {} items", updated_items);
        Ok(())
    }
} 
