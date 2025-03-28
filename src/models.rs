use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use crate::tag::{Tag, TagRule};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub last_updated: Option<DateTime<Utc>>,
}

impl TagRule for Feed {
    fn find_tag(&self, item: &FeedItem) -> Option<Tag> {
        if item.feed_id == self.id {
            Some(Tag { // Tag format: feed/title
                name: format!("feed/{}", self.title.as_ref().unwrap_or(&"unknown".to_string())),
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FeedItem {
    pub feed_id: i64,           // Won't be stored in the database, but used for rules
    pub tags: HashSet<String>,  // Should be a set, but the implementation is not serializable, so we use HashSet here
    pub title: String,
    pub url: String,
    pub content: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
} 
