use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A feed source with its associated tags
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub tags: HashSet<String>, // Tags inherited by all articles
    pub last_updated: Option<DateTime<Utc>>,
}

/// An article from a feed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub id: i64,
    pub source_id: i64,
    pub url: String,
    pub title: String,
    pub content: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub word_count: u32,
    pub tags: HashSet<String>, // Inherited from source + rule-added
    pub read: bool,
}

impl Article {
    /// Check if any of article's tags match the query tag (hierarchically).
    ///
    /// Hierarchical matching: query "tech" matches tags "tech", "tech/ai", "tech/ai/llm"
    /// but NOT "llm" or "ai" alone.
    ///
    /// Examples (article has tag "tech/ai/llm"):
    /// - query "tech/ai/llm" -> true  (exact match)
    /// - query "tech/ai"     -> true  (prefix match)
    /// - query "tech"        -> true  (prefix match)
    /// - query "llm"         -> false (suffix, not prefix)
    /// - query "ai"          -> false (middle segment, not prefix)
    ///
    /// Examples (article has tag "ai"):
    /// - query "tech/ai"     -> false (query is longer than tag)
    pub fn match_tag(&self, query: &str) -> bool {
        self.tags.iter().any(|article_tag| {
            // Exact match
            if article_tag == query {
                return true;
            }
            // Hierarchical: article_tag starts with query/
            // e.g., article_tag="tech/ai/llm", query="tech" -> "tech/ai/llm".starts_with("tech/")
            let prefix = format!("{}/", query);
            article_tag.starts_with(&prefix)
        })
    }
}

/// A tagging rule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Rule {
    /// Add tag if content/title contains pattern
    Contains {
        pattern: String,
        #[serde(default)]
        case_sensitive: bool,
        tag: String,
    },
    /// Add tag if word count matches condition
    WordCount {
        min: Option<u32>,
        max: Option<u32>,
        tag: String,
    },
    /// Add tag if published within time range
    Age {
        max_days: Option<u32>,
        min_days: Option<u32>,
        tag: String,
    },
}

impl Rule {
    pub fn apply(&self, article: &Article) -> Option<String> {
        match self {
            Rule::Contains {
                pattern,
                case_sensitive,
                tag,
            } => {
                let haystack = format!(
                    "{} {}",
                    &article.title,
                    article.content.as_deref().unwrap_or("")
                );
                let matches = if *case_sensitive {
                    haystack.contains(pattern)
                } else {
                    haystack.to_lowercase().contains(&pattern.to_lowercase())
                };
                if matches {
                    Some(tag.clone())
                } else {
                    None
                }
            }
            Rule::WordCount { min, max, tag } => {
                let wc = article.word_count;
                let above_min = min.map_or(true, |m| wc >= m);
                let below_max = max.map_or(true, |m| wc <= m);
                if above_min && below_max {
                    Some(tag.clone())
                } else {
                    None
                }
            }
            Rule::Age {
                max_days,
                min_days,
                tag,
            } => {
                let Some(pub_date) = article.published_at else {
                    return None;
                };
                let age_days = (Utc::now() - pub_date).num_days() as u32;
                let recent_enough = max_days.map_or(true, |m| age_days <= m);
                let old_enough = min_days.map_or(true, |m| age_days >= m);
                if recent_enough && old_enough {
                    Some(tag.clone())
                } else {
                    None
                }
            }
        }
    }
}
