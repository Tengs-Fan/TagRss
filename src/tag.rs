use std::fs;
use std::path::Path;
use std::io::{Read, Write};
use anyhow::Result;
use serde::{Serialize, Deserialize};
use regex::Regex;
use chrono::{DateTime, Utc};
use log::{info, warn, debug, error};
use crate::models::{FeedItem, Feed};

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
            info!("Tag rules file does not exist, creating new file");
            // Create the file with empty content
            match fs::File::create(file_path) {
                Ok(_) => debug!("Created new tag rules file at: {}", file_path),
                Err(e) => error!("Failed to create tag rules file: {}", e),
            }
            return Vec::new();
        }
        
        let mut file = match fs::File::open(file_path) {
            Ok(file) => file,
            Err(e) => {
                error!("Failed to open tag rules file: {}", e);
                return Vec::new();
            }
        };
        
        let mut contents = String::new();
        if let Err(e) = file.read_to_string(&mut contents) {
            error!("Failed to read tag rules content: {}", e);
            return Vec::new();
        }
        
        match serde_json::from_str(&contents) {
            Ok(loaded) => {
                let loaded: TagManager = loaded;
                debug!("Loaded {} tag rules from file", loaded.rules.len());
                loaded.rules
            },
            Err(e) => {
                error!("Failed to parse tag rules JSON: {}", e);
                Vec::new()
            }
        }
    }

    pub fn save_to_file(&self) -> Result<()> {
        let json = serde_json::to_string(&self)?;
        let mut file = fs::File::create(&self.file_path)?;
        file.write_all(json.as_bytes())?;
        debug!("Saved {} tag rules to file", self.rules.len());
        Ok(())
    }

    pub fn add_rule(&mut self, rule: TagRuleEnum) {
        self.rules.push(rule);
        debug!("Added new rule, total rules: {}", self.rules.len());
    }

    pub fn rules(&self) -> &Vec<TagRuleEnum> {
        &self.rules
    }

    pub fn apply_rules(&self, feed_item: &mut FeedItem) -> Result<()> {
        let initial_tag_count = feed_item.tags.len();
        for rule in &self.rules {
            if let Some(tag) = rule.find_tag(feed_item) {
                feed_item.tags.insert(tag.name);
            }
        }
        let new_tag_count = feed_item.tags.len() - initial_tag_count;
        if new_tag_count > 0 {
            debug!("Applied {} tags to item: {}", new_tag_count, feed_item.title);
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
    FromFeed(Feed),
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
    pub target_regex: String,
}

impl TagRule for Contains {
    fn find_tag(&self, feed: &FeedItem) -> Option<Tag> {
        let target_regex = Regex::new(&self.target_regex).unwrap();

        // Check title with regex
        if target_regex.is_match(&feed.title) {
            return Some(self.tag.clone());
        }
        
        // Also check content if available
        if let Some(content) = &feed.content {
            if target_regex.is_match(content) {
                return Some(self.tag.clone());
            }
        }
        
        None
    }
}
