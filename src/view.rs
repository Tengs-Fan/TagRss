//! What the user sees: stored entries joined with the config and the clock into tagged items.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};

use crate::config::{Config, Folder, Source};
use crate::db::{Database, Entry, Fetch};
use crate::tags::{self, Facts};

pub struct Item {
    pub entry: Entry,
    pub source_title: String,
    pub tags: HashSet<String>,
    pub read: bool,
}

/// A source's display title: the config's title, else the feed's own, else its URL.
pub fn source_title(url: &str, source: Option<&Source>, fetch: Option<&Fetch>) -> String {
    source
        .and_then(|s| s.title.clone())
        .or_else(|| fetch.and_then(|f| f.feed_title.clone()))
        .unwrap_or_else(|| url.to_string())
}

/// Every stored entry with its tags as of `now`, newest first.
pub fn items(config: &Config, db: &Database, now: DateTime<Utc>) -> Result<Vec<Item>> {
    let sources: HashMap<&str, &Source> = config.sources.iter().map(|s| (s.url.as_str(), s)).collect();
    let fetches = db.fetches()?;
    let rebuilt_at = db.rebuilt_at()?;
    let no_tags: Vec<String> = Vec::new();

    let mut items: Vec<Item> = db
        .entries()?
        .into_iter()
        .map(|entry| {
            // A source removed from the config keeps its entries, with system tags only.
            let source = sources.get(entry.source_url.as_str()).copied();
            let title = source_title(&entry.source_url, source, fetches.get(&entry.source_url));
            let read = tags::is_read(entry.mark, entry.published_at, entry.first_seen_at, rebuilt_at);
            let facts = Facts {
                source_tags: source.map_or(&no_tags, |s| &s.tags),
                source_title: &title,
                published_at: entry.published_at,
                first_seen_at: entry.first_seen_at,
                word_count: entry.word_count,
                read,
            };
            let tags = tags::compute(&facts, now);
            Item { entry, source_title: title, tags, read }
        })
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.entry.published_at.unwrap_or(i.entry.first_seen_at)));
    Ok(items)
}

pub fn in_folder<'a>(items: &'a [Item], folder: &'a Folder) -> impl Iterator<Item = &'a Item> {
    items.iter().filter(|i| folder.rule.matches(&i.tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::db::NewEntry;
    use chrono::{Duration, TimeZone};

    #[test]
    fn folders_select_by_source_and_system_tags() {
        let now = Utc.with_ymd_and_hms(2026, 10, 10, 12, 0, 0).unwrap();
        let cfg = config::parse(
            r#"
            [[source]]
            url = "https://ai.example/feed"
            title = "AI Blog"
            tags = ["topic/ai"]

            [[source]]
            url = "https://misc.example/feed"

            [[folder]]
            name = "AI unread"
            rule = "topic/ai AND state/unread"

            [[folder]]
            name = "Needs tags"
            rule = "NOT topic"
        "#,
        )
        .unwrap()
        .config;
        let db = Database::open_in_memory(Some(now - Duration::days(5))).unwrap();
        let add = |source: &str, key: &str, days_ago: i64| {
            db.insert_entry(
                &NewEntry {
                    source_url: source,
                    entry_key: key,
                    url: None,
                    title: key,
                    published_at: Some(now - Duration::days(days_ago)),
                    word_count: Some(10),
                },
                now,
            )
            .unwrap();
        };
        add("https://ai.example/feed", "ai-new", 1);
        add("https://ai.example/feed", "ai-old", 9); // before rebuilt_at: counts as read
        add("https://misc.example/feed", "misc", 1);
        add("https://gone.example/feed", "gone", 1); // source no longer in the config

        let items = items(&cfg, &db, now).unwrap();
        let names = |folder: &Folder| in_folder(&items, folder).map(|i| i.entry.title.clone()).collect::<Vec<_>>();
        assert_eq!(names(&cfg.folders[0]), vec!["ai-new"]);
        let mut untagged = names(&cfg.folders[1]);
        untagged.sort();
        assert_eq!(untagged, vec!["gone", "misc"]);

        let ai_new = items.iter().find(|i| i.entry.title == "ai-new").unwrap();
        assert!(ai_new.tags.contains("source/ai-blog"));
        let gone = items.iter().find(|i| i.entry.title == "gone").unwrap();
        assert_eq!(gone.source_title, "https://gone.example/feed");

        // Explicitly marking the old entry unread overrides the rebuild rule.
        let old_id = items.iter().find(|i| i.entry.title == "ai-old").unwrap().entry.id;
        db.set_mark(old_id, false, now).unwrap();
        let items_after = super::items(&cfg, &db, now).unwrap();
        let mut ai = in_folder(&items_after, &cfg.folders[0]).map(|i| i.entry.title.clone()).collect::<Vec<_>>();
        ai.sort();
        assert_eq!(ai, vec!["ai-new", "ai-old"]);
    }
}
