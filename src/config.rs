use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::folder::{Expr, Folder};
use crate::models::Rule;

/// A feed entry parsed from OPML
#[derive(Debug)]
pub struct OpmlFeed {
    pub title: String,
    pub xml_url: String,
    pub html_url: Option<String>,
    pub tags: HashSet<String>,
}

/// Parse OPML file and extract feeds with tags
pub fn load_opml(path: impl AsRef<Path>) -> Result<Vec<OpmlFeed>> {
    let content = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read OPML file: {:?}", path.as_ref()))?;

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut feeds = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) if e.name().as_ref() == b"outline" => {
                if let Some(feed) = parse_outline(e) {
                    feeds.push(feed);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("XML parse error: {}", e)),
            _ => {}
        }
    }

    Ok(feeds)
}

fn parse_outline(e: &BytesStart) -> Option<OpmlFeed> {
    let mut feed_type = None;
    let mut text = None;
    let mut xml_url = None;
    let mut html_url = None;
    let mut tags_str = None;

    for attr in e.attributes().filter_map(|a| a.ok()) {
        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
        let value = String::from_utf8_lossy(&attr.value).to_string();

        match key {
            "type" => feed_type = Some(value),
            "text" | "title" => {
                if text.is_none() {
                    text = Some(value);
                }
            }
            "xmlUrl" => xml_url = Some(value),
            "htmlUrl" => html_url = Some(value),
            "tags" => tags_str = Some(value),
            _ => {}
        }
    }

    // Only process RSS/Atom feeds (type="rss" or has xmlUrl)
    let xml_url = xml_url?;
    if feed_type.as_deref() != Some("rss") && feed_type.as_deref() != Some("atom") {
        // Some OPML files don't set type, but have xmlUrl
        if feed_type.is_some() {
            return None;
        }
    }

    let title = text.unwrap_or_else(|| xml_url.clone());
    let tags: HashSet<String> = tags_str
        .map(|s: String| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Some(OpmlFeed {
        title,
        xml_url,
        html_url,
        tags,
    })
}

/// YAML structure for rules file
#[derive(Debug, Deserialize)]
pub struct RulesConfig {
    pub rules: Vec<RuleConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum RuleConfig {
    #[serde(rename = "word_count")]
    WordCount {
        min: Option<u32>,
        max: Option<u32>,
        tag: String,
    },
    #[serde(rename = "contains")]
    Contains {
        pattern: String,
        #[serde(default)]
        case_sensitive: bool,
        tag: String,
    },
    #[serde(rename = "age")]
    Age {
        max_days: Option<u32>,
        min_days: Option<u32>,
        tag: String,
    },
}

impl From<RuleConfig> for Rule {
    fn from(config: RuleConfig) -> Self {
        match config {
            RuleConfig::WordCount { min, max, tag } => Rule::WordCount { min, max, tag },
            RuleConfig::Contains { pattern, case_sensitive, tag } => {
                Rule::Contains { pattern, case_sensitive, tag }
            }
            RuleConfig::Age { max_days, min_days, tag } => Rule::Age { max_days, min_days, tag },
        }
    }
}

/// Load rules from YAML file
pub fn load_rules(path: impl AsRef<Path>) -> Result<Vec<Rule>> {
    let content = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read rules file: {:?}", path.as_ref()))?;

    let config: RulesConfig = serde_yaml::from_str(&content)
        .with_context(|| "Failed to parse rules YAML")?;

    Ok(config.rules.into_iter().map(Rule::from).collect())
}

/// YAML structure for folders file
#[derive(Debug, Deserialize)]
pub struct FoldersConfig {
    pub folders: Vec<FolderConfig>,
}

#[derive(Debug, Deserialize)]
pub struct FolderConfig {
    pub name: String,
    pub filter: String,
}

/// Load folders from YAML file
pub fn load_folders(path: impl AsRef<Path>) -> Result<Vec<Folder>> {
    let content = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read folders file: {:?}", path.as_ref()))?;

    let config: FoldersConfig = serde_yaml::from_str(&content)
        .with_context(|| "Failed to parse folders YAML")?;

    let mut folders = Vec::new();
    for fc in config.folders {
        let filter = Expr::parse(&fc.filter)
            .map_err(|e| anyhow::anyhow!("Failed to parse filter for '{}': {}", fc.name, e))?;
        folders.push(Folder {
            name: fc.name,
            filter,
        });
    }

    Ok(folders)
}
