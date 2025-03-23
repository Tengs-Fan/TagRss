use std::ptr::eq;
use std::fs;
use std::path::Path;
use std::io::{Read, Write};
use anyhow::Result;
use serde::{Serialize, Deserialize};

use chrono::{DateTime, Utc};
use crate::models::FeedItem;

#[derive(Debug, Serialize, Deserialize)]
pub struct TagManager {
    rules: Vec<TagRuleEnum>,
    file_path: String,
}

impl TagManager {
    pub fn new(file_path: &str) -> Self {
        Self {
            rules: Vec::new(),
            file_path: file_path.to_string(),
        }
    }

    pub fn add_rule(&mut self, rule: TagRuleEnum) {
        self.rules.push(rule);
    }

    pub fn rules(&self) -> &Vec<TagRuleEnum> {
        &self.rules
    }

    pub fn apply_rules(&self, feed_item: &mut FeedItem) -> Result<()> {
        for rule in &self.rules {
            if let Some(tag) = rule.find_tag(feed_item) {
                // Add the tag's ID to the feed item if not already present
                if !feed_item.tags.contains(&tag.id) {
                    feed_item.tags.push(tag.id);
                }
            }
        }
        Ok(())
    }

    pub fn save_to_file(&self) -> Result<()> {
        let json = serde_json::to_string(&self)?;
        let mut file = fs::File::create(&self.file_path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn load_from_file(&mut self) -> Result<()> {
        if !Path::new(&self.file_path).exists() {
            return Ok(());
        }
        
        let mut file = fs::File::open(&self.file_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        
        let loaded: TagManager = serde_json::from_str(&contents)?;
        self.rules = loaded.rules;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum TagRuleEnum {
    TimeRange(TimeRange),
    Contains(Contains),
    FromFeed(FromFeed),
}

impl TagRuleEnum {
    pub fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        match self {
            TagRuleEnum::TimeRange(rule) => rule.find_tag(feed),
            TagRuleEnum::Contains(rule) => rule.find_tag(feed),
            TagRuleEnum::FromFeed(rule) => rule.find_tag(feed),
        }
    }
}

pub trait TagRule {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag>;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TimeRange {
    pub name: String,
    pub tag_id: i64,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

impl TagRule for TimeRange {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        match (self.start, self.end, feed.published_at) {
            (_, _, None) => None, // If feed has no publish date, cannot match
            (None, None, _) => {
                // Invalid rule configuration
                None
            }
            (Some(start), None, Some(published)) => {
                if published >= start {
                    Some(Tag {
                        id: self.tag_id,
                        name: self.name.clone(),
                    })
                } else {
                    None
                }
            }
            (None, Some(end), Some(published)) => {
                if published <= end {
                    Some(Tag {
                        id: self.tag_id,
                        name: self.name.clone(),
                    })
                } else {
                    None
                }
            }
            (Some(start), Some(end), Some(published)) => {
                if published >= start && published <= end {
                    Some(Tag {
                        id: self.tag_id,
                        name: self.name.clone(),
                    })
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Contains {
    pub name: String,
    pub tag_id: i64,
    pub target_string: String,
    pub case_sensitive: bool,
}

impl TagRule for Contains {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        let title = if self.case_sensitive {
            feed.title.clone()
        } else {
            feed.title.to_lowercase()
        };
        
        let target = if self.case_sensitive {
            self.target_string.clone()
        } else {
            self.target_string.to_lowercase()
        };
        
        if title.contains(&target) {
            Some(Tag {
                id: self.tag_id,
                name: self.name.clone(),
            })
        } else {
            // Also check content if available
            if let Some(content) = &feed.content {
                let content = if self.case_sensitive {
                    content.clone()
                } else {
                    content.to_lowercase()
                };
                
                if content.contains(&target) {
                    return Some(Tag {
                        id: self.tag_id,
                        name: self.name.clone(),
                    });
                }
            }
            None
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FromFeed {
    pub name: String,
    pub tag_id: i64,
    pub feed_id: i64,
}

impl TagRule for FromFeed {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        // A more complete implementation would compare with feed.feed_id
        // which isn't currently part of the FeedItem struct
        if feed.id == self.feed_id {
            Some(Tag {
                id: self.tag_id,
                name: self.name.clone(),
            })
        } else {
            None
        }
    }
}
