use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub last_updated: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FeedItem {
    pub id: i64,
    pub tags: Vec<i64>,
    pub title: String,
    pub url: String,
    pub content: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
} 
