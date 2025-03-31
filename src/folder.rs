use crate::tag::{Tag, TagRuleEnum, TimeRange, Contains, TagRule};
use crate::models::FeedItem;
use std::fs;
use std::path::Path;
use std::io::Read;
use anyhow::{Result, anyhow};
use log::{info, warn, debug, error};
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use regex::Regex;

#[derive(Debug, Serialize, Deserialize)]
pub enum Rules {
    NODE(RulesNode),
    LEAF(RulesLeaf),
}

pub trait Rule {
    fn evaluate(&self, item: &FeedItem) -> bool;
}

impl Rules {
    pub fn evaluate(&self, feed: &FeedItem) -> bool {
        match self {
            Rules::NODE(rule) => rule.evaluate(feed),
            Rules::LEAF(rule) => rule.evaluate(feed),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RulesLeaf {
    reverse: bool,  // NOT operator
    rule_type: TagRuleEnum,
}

impl Rule for RulesLeaf {
    fn evaluate(&self, item: &FeedItem) -> bool {
        // Use the TagRule trait's find_tag method which already knows how to evaluate each rule type
        let result = self.rule_type.find_tag(item).is_some();

        if self.reverse {
            !result
        } else {
            result
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RulesNode {
    rules: Vec<Rules>,
    pub is_and: bool, // true for AND, false for OR
}

impl Rule for RulesNode {
    fn evaluate(&self, item: &FeedItem) -> bool {
        if self.is_and {
            // AND logic: all rules must evaluate to true
            self.rules.iter().all(|r| r.evaluate(item))
        } else {
            // OR logic: at least one rule must evaluate to true
            self.rules.iter().any(|r| r.evaluate(item))
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub root: Rules,
}

/// Manages a collection of folders
#[derive(Debug, Serialize, Deserialize)]
pub struct FolderManager {
    /// List of folders
    pub folders: Vec<Folder>,
    /// Path to the YAML configuration file
    config_path: String,
}

impl FolderManager {
    /// Create a new folder manager with the given config file path
    pub fn new(config_path: &str) -> Self {
        let folders = if Path::new(config_path).exists() {
            match Self::load_yaml_config(config_path) {
                Ok(folders) => folders,
                Err(e) => {
                    error!("Failed to load folder configuration: {}", e);
                    Vec::new()
                }
            }
        } else {
            info!("Folder configuration file does not exist: {}", config_path);
            Vec::new()
        };
        
        Self {
            folders,
            config_path: config_path.to_string(),
        }
    }
    
    /// Load folder configuration from YAML file
    pub fn load_yaml_config(yaml_path: &str) -> Result<Vec<Folder>> {
        info!("Loading folders from YAML file: {}", yaml_path);
        
        if !Path::new(yaml_path).exists() {
            return Err(anyhow!("YAML file not found: {}", yaml_path));
        }
        
        let mut file = fs::File::open(yaml_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        
        let yaml_config: YamlConfig = serde_yaml::from_str(&contents)?;
        
        let mut folders = Vec::new();
        let mut converted_count = 0;
        
        for yaml_folder in yaml_config.folders {
            match Self::convert_yaml_folder_to_folder(&yaml_folder) {
                Ok(folder) => {
                    debug!("Loaded folder: {}", folder.name);
                    folders.push(folder);
                    converted_count += 1;
                },
                Err(e) => {
                    warn!("Failed to convert folder '{}': {}", yaml_folder.name, e);
                }
            }
        }
        
        info!("Loaded {} folders from YAML file", converted_count);
        Ok(folders)
    }
    
    /// Reload the folder configuration from the config file
    pub fn reload_config(&mut self) -> Result<()> {
        self.folders = Self::load_yaml_config(&self.config_path)?;
        Ok(())
    }
    
    /// Convert a YAML folder to an internal Folder struct
    fn convert_yaml_folder_to_folder(yaml_folder: &YamlFolder) -> Result<Folder> {
        let rules = Self::parse_yaml_rule_item(&yaml_folder.rule)?;
        
        Ok(Folder {
            name: yaml_folder.name.clone(),
            root: rules,
        })
    }
    
    /// Parse a YAML rule item into our internal Rules enum
    fn parse_yaml_rule_item(rule_item: &YamlRuleItem) -> Result<Rules> {
        match rule_item {
            YamlRuleItem::Tag { tag } => {
                // Tag rule
                Ok(Rules::LEAF(RulesLeaf {
                    reverse: false,
                    rule_type: TagRuleEnum::Contains(Contains {
                        tag: Tag::new(tag.clone()),
                        target_regex: tag.clone(), // Exact match
                    }),
                }))
            },
            YamlRuleItem::Time { time } => {
                // Time range rule
                let (start_date, end_date) = Self::parse_time_range(time)?;
                Ok(Rules::LEAF(RulesLeaf {
                    reverse: false,
                    rule_type: TagRuleEnum::TimeRange(TimeRange {
                        tag: Tag::new("time_range".to_string()),
                        start: start_date,
                        end: end_date,
                    }),
                }))
            },
            YamlRuleItem::Contains { contains } => {
                // Contains rule (for title/content search)
                Ok(Rules::LEAF(RulesLeaf {
                    reverse: false,
                    rule_type: TagRuleEnum::Contains(Contains {
                        tag: Tag::new("content_match".to_string()),
                        target_regex: contains.clone(),
                    }),
                }))
            },
            YamlRuleItem::Not { not } => {
                let child_rule = Self::parse_yaml_rule_item(not)?;
                
                match child_rule {
                    Rules::LEAF(leaf) => {
                        Ok(Rules::LEAF(RulesLeaf {
                            reverse: true,
                            rule_type: leaf.rule_type,
                        }))
                    },
                    _ => Err(anyhow!("NOT operation currently only supports simple rules")),
                }
            },
            YamlRuleItem::And { and } => {
                let mut child_rules = Vec::new();
                for item in and {
                    child_rules.push(Self::parse_yaml_rule_item(item)?);
                }
                
                Ok(Rules::NODE(RulesNode {
                    is_and: true, // Use AND logic
                    rules: child_rules,
                }))
            },
            YamlRuleItem::Or { or } => {
                let mut child_rules = Vec::new();
                for item in or {
                    child_rules.push(Self::parse_yaml_rule_item(item)?);
                }
                
                Ok(Rules::NODE(RulesNode {
                    is_and: false, // Use OR logic
                    rules: child_rules,
                }))
            },
        }
    }
    
    /// Parse a time range rule from a string like "2024-01-01 ~ 2024-01-31"
    /// Also supports special formats like "Yesterday ~ " (yesterday to open-ended future)
    fn parse_time_range(time_str: &str) -> Result<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> {
        // Split the string by '~' to get the start and end dates
        let parts: Vec<&str> = time_str.split('~').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid time range format. Expected 'start ~ end' but got: {}", time_str));
        }
        
        let start_str = parts[0].trim();
        let end_str = parts[1].trim();
        
        // Parse the start date, handling special formats
        let start_date = if start_str.is_empty() {
            None
        } else if start_str.eq_ignore_ascii_case("yesterday") {
            // Get yesterday's date
            let yesterday = Utc::now().date_naive().pred_opt().unwrap();
            Some(DateTime::<Utc>::from_naive_utc_and_offset(
                yesterday.and_hms_opt(0, 0, 0).unwrap(),
                Utc,
            ))
        } else if start_str.eq_ignore_ascii_case("today") {
            // Get today's date
            let today = Utc::now().date_naive();
            Some(DateTime::<Utc>::from_naive_utc_and_offset(
                today.and_hms_opt(0, 0, 0).unwrap(),
                Utc,
            ))
        } else if start_str.eq_ignore_ascii_case("tomorrow") {
            // Get tomorrow's date
            let tomorrow = Utc::now().date_naive().succ_opt().unwrap();
            Some(DateTime::<Utc>::from_naive_utc_and_offset(
                tomorrow.and_hms_opt(0, 0, 0).unwrap(),
                Utc,
            ))
        } else {
            // Parse as a regular date
            match chrono::NaiveDate::parse_from_str(start_str, "%Y-%m-%d") {
                Ok(date) => Some(DateTime::<Utc>::from_naive_utc_and_offset(
                    date.and_hms_opt(0, 0, 0).unwrap(),
                    Utc,
                )),
                Err(e) => return Err(anyhow!("Failed to parse start date '{}': {}", start_str, e)),
            }
        };
        
        // Parse the end date
        let end_date = if end_str.is_empty() {
            None // Open-ended (no upper bound)
        } else if end_str.eq_ignore_ascii_case("yesterday") {
            // Get yesterday's date
            let yesterday = Utc::now().date_naive().pred_opt().unwrap();
            Some(DateTime::<Utc>::from_naive_utc_and_offset(
                yesterday.and_hms_opt(23, 59, 59).unwrap(),
                Utc,
            ))
        } else if end_str.eq_ignore_ascii_case("today") {
            // Get today's date
            let today = Utc::now().date_naive();
            Some(DateTime::<Utc>::from_naive_utc_and_offset(
                today.and_hms_opt(23, 59, 59).unwrap(),
                Utc,
            ))
        } else if end_str.eq_ignore_ascii_case("tomorrow") {
            // Get tomorrow's date
            let tomorrow = Utc::now().date_naive().succ_opt().unwrap();
            Some(DateTime::<Utc>::from_naive_utc_and_offset(
                tomorrow.and_hms_opt(23, 59, 59).unwrap(),
                Utc,
            ))
        } else {
            match chrono::NaiveDate::parse_from_str(end_str, "%Y-%m-%d") {
                Ok(date) => Some(DateTime::<Utc>::from_naive_utc_and_offset(
                    // Set to end of day
                    date.and_hms_opt(23, 59, 59).unwrap(),
                    Utc,
                )),
                Err(e) => return Err(anyhow!("Failed to parse end date '{}': {}", end_str, e)),
            }
        };
        
        Ok((start_date, end_date))
    }
    
    // Parse time rule (obsolete - redirects to parse_time_range)
    fn parse_time_rule(time_str: &str) -> Result<Rules> {
        let (start_date, end_date) = Self::parse_time_range(time_str)?;
        Ok(Rules::LEAF(RulesLeaf {
            reverse: false,
            rule_type: TagRuleEnum::TimeRange(TimeRange {
                tag: Tag::new("time_range".to_string()),
                start: start_date,
                end: end_date,
            }),
        }))
    }
}

// YAML configuration structures
#[derive(Debug, Serialize, Deserialize)]
struct YamlConfig {
    folders: Vec<YamlFolder>,
}

#[derive(Debug, Serialize, Deserialize)]
struct YamlFolder {
    name: String,
    #[serde(flatten)]
    rule: YamlRuleItem,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum YamlRuleItem {
    Tag { tag: String },
    Time { time: String },
    Contains { contains: String },
    Not { 
        not: Box<YamlRuleItem>
    },
    // AND with multiple items
    And { 
        and: Vec<YamlRuleItem>
    },
    // OR with multiple items
    Or { 
        or: Vec<YamlRuleItem>
    },
}
