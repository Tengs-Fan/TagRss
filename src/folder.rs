use crate::tag::{Tag, TimeRange};
use crate::models::FeedItem;
use std::fs;
use std::path::Path;
use std::io::Read;
use anyhow::{Result, anyhow};
use log::{info, warn, debug, error};
use serde::{Serialize, Deserialize};
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum RuleType {
    TitleContains(String),
    ContentContains(String),
    TimeRange(TimeRange),
    FeedTag(Tag),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RulesLeaf {
    reverse: bool,  // NOT operator
    rule: RuleType,
}

impl Rule for RulesLeaf {
    fn evaluate(&self, item: &FeedItem) -> bool {
        // Use the TagRule trait's find_tag method which already knows how to evaluate each rule type
        let result = match &self.rule {
            RuleType::TitleContains(regex) => {
                let target_regex = Regex::new(&regex).unwrap();
                target_regex.is_match(&item.title)
            }
            RuleType::ContentContains(regex) => {
                let target_regex = Regex::new(&regex).unwrap();
                target_regex.is_match(&item.content.as_ref().unwrap())
            }
            RuleType::FeedTag(tag) => {
                unimplemented!()
            }
            RuleType::TimeRange(time_range) => {
                let published = item.published_at.unwrap();
                return published >= time_range.start.to_datetime().unwrap() && published <= time_range.end.to_datetime().unwrap()
            }
        };
        self.reverse != result
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

impl Folder {
    /// Find all items that match the folder's rules
    pub fn find_matching_items<'a>(&self, items: &'a [FeedItem]) -> Vec<&'a FeedItem> {
        items.iter().filter(|item| self.root.evaluate(item)).collect()
    }
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
                Ok(Rules::LEAF(RulesLeaf {
                    reverse: false,
                    rule: RuleType::FeedTag(Tag::new(tag.clone())),
                }))
            },
            YamlRuleItem::Time { time } => {
                Ok(Rules::LEAF(RulesLeaf {
                    reverse: false,
                    rule: RuleType::TimeRange(
                        TimeRange::new(time)?
                    ),
                }))
            },
            YamlRuleItem::TitleContains { title_contains } => {
                Ok(Rules::LEAF(RulesLeaf {
                    reverse: false,
                    rule: RuleType::TitleContains(title_contains.clone()),
                }))
            },
            YamlRuleItem::ContentContains { content_contains } => {
                Ok(Rules::LEAF(RulesLeaf {
                    reverse: false,
                    rule: RuleType::ContentContains(content_contains.clone()),
                }))
            },
            YamlRuleItem::Not { not } => {
                let child_rule = Self::parse_yaml_rule_item(not)?;
                
                match child_rule {
                    Rules::LEAF(leaf) => {
                        Ok(Rules::LEAF(RulesLeaf {
                            reverse: true,
                            rule: leaf.rule,
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
            }
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
    TitleContains { title_contains: String },
    ContentContains { content_contains: String },
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
