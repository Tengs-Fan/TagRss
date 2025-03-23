use std::fs;
use std::path::Path;
use std::io::{Read, Write};
use anyhow::Result;
use serde::{Serialize, Deserialize};
use regex::Regex;
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
            rules: Self::load_from_file(file_path),
            file_path: file_path.to_string(),
        }
    }

    fn load_from_file(file_path: &str) -> Vec<TagRuleEnum> {
        if !Path::new(file_path).exists() {
            println!("File does not exist");
            return Vec::new();
        }
        
        let mut file = fs::File::open(file_path).expect("Failed to open file");
        let mut contents = String::new();
        file.read_to_string(&mut contents).expect("Failed to read content");
        
        let loaded: TagManager = serde_json::from_str(&contents).expect("Failed to parse JSON");
        loaded.rules
    }

    pub fn save_to_file(&self) -> Result<()> {
        let json = serde_json::to_string(&self)?;
        let mut file = fs::File::create(&self.file_path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
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
                feed_item.tags.insert(tag.name);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tag {
    // The name is a tree structure, e.g. "Technology > Artificial Intelligence"
    // The parent and children are separated by "/"
    // Example: "tech/ai", "math/affine", "tech/ai/machine-learning", "math/affine/linear-algebra"
    pub name: String,
}

impl Tag {
    pub fn new(name: String) -> Self {
        Self { name }
    }
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
    pub tag: Tag,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

impl TagRule for TimeRange {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        match (self.start, self.end, feed.published_at) {
            (_, _, None) => None, // If feed has no publish date, cannot match
            (None, None, _) => { // Invalid rule configuration
                None
            }
            (Some(start), None, Some(published)) => {
                if published >= start {
                    Some(self.tag.clone())
                } else {
                    None
                }
            }
            (None, Some(end), Some(published)) => {
                if published <= end {
                    Some(self.tag.clone())
                } else {
                    None
                }
            }
            (Some(start), Some(end), Some(published)) => {
                if published >= start && published <= end {
                    Some(self.tag.clone())
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Contains {
    pub tag: Tag,
    // Use a regex to match the target string
    pub target_regex: Regex,
}

impl TagRule for Contains {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        // Check title with regex
        if self.target_regex.is_match(&feed.title) {
            return Some(self.tag.clone());
        }
        
        // Also check content if available
        if let Some(content) = &feed.content {
            if self.target_regex.is_match(content) {
                return Some(self.tag.clone());
            }
        }
        
        None
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FromFeed {
    pub tag: Tag,
    pub feed_id: i64,
}

impl TagRule for FromFeed {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        // A more complete implementation would compare with feed.feed_id
        // which isn't currently part of the FeedItem struct
        if feed.id == self.feed_id {
            Some(self.tag.clone())
        } else {
            None
        }
    }
}
