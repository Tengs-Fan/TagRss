use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

use crate::db::Database;
use crate::models::{Article, Rule, Source};

/// Fetch and parse a feed from URL
pub async fn fetch_feed(url: &str) -> Result<(String, Vec<RawEntry>)> {
    let response = reqwest::get(url).await?.bytes().await?;
    let feed = feed_rs::parser::parse(&response[..])?;

    let title = feed
        .title
        .map(|t| t.content)
        .unwrap_or_else(|| "Untitled".to_string());

    let entries: Vec<RawEntry> = feed
        .entries
        .into_iter()
        .map(|e| {
            let entry_url = e
                .links
                .first()
                .map(|l| l.href.clone())
                .or_else(|| Some(e.id.clone()))
                .unwrap_or_default();

            let content = e
                .content
                .and_then(|c| c.body)
                .or_else(|| e.summary.map(|s| s.content));

            let word_count = content.as_ref().map(|c| count_words(c)).unwrap_or(0);

            let published = e.published.or(e.updated);

            RawEntry {
                url: entry_url,
                title: e
                    .title
                    .map(|t| t.content)
                    .unwrap_or_else(|| "Untitled".to_string()),
                content,
                published_at: published,
                word_count,
            }
        })
        .filter(|e| !e.url.is_empty())
        .collect();

    Ok((title, entries))
}

#[derive(Debug)]
pub struct RawEntry {
    pub url: String,
    pub title: String,
    pub content: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub word_count: u32,
}

fn count_words(html: &str) -> u32 {
    // Simple word count: strip HTML tags, count whitespace-separated tokens
    let text = html
        .split('<')
        .flat_map(|s| s.split('>').skip(1))
        .collect::<Vec<_>>()
        .join(" ");
    text.split_whitespace().count() as u32
}

/// Sync a source: fetch feed and add new articles
pub async fn sync_source(db: &Database, source: &Source, rules: &[(i64, Rule)]) -> Result<u32> {
    let (_, entries) = fetch_feed(&source.url).await?;
    let mut added = 0;

    for entry in entries {
        if db.article_exists(&entry.url)? {
            continue;
        }

        // Start with tags inherited from source
        let mut tags = source.tags.clone();

        // Create article (temporarily without final tags for rule evaluation)
        let mut article = Article {
            id: 0,
            source_id: source.id,
            url: entry.url,
            title: entry.title,
            content: entry.content,
            published_at: entry.published_at,
            word_count: entry.word_count,
            tags: HashSet::new(),
            read: false,
        };

        // Apply rules
        for (_, rule) in rules {
            if let Some(tag) = rule.apply(&article) {
                tags.insert(tag);
            }
        }

        article.tags = tags;
        db.add_article(&article)?;
        added += 1;
    }

    db.update_source_timestamp(source.id, Utc::now())?;
    Ok(added)
}
