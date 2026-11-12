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
    let entries = db.entries()?;

    // When each source first delivered anything; see `tags::published`.
    let mut first_seen: HashMap<&str, DateTime<Utc>> = HashMap::new();
    for e in &entries {
        let first = first_seen.entry(e.source_url.as_str()).or_insert(e.first_seen_at);
        *first = (*first).min(e.first_seen_at);
    }
    let published: Vec<Option<DateTime<Utc>>> = entries
        .iter()
        .map(|e| tags::published(e.published_at, e.first_seen_at, first_seen[e.source_url.as_str()]))
        .collect();

    let mut items: Vec<Item> = entries
        .into_iter()
        .zip(published)
        .map(|(entry, published)| {
            // A source removed from the config keeps its entries, with system tags only.
            let source = sources.get(entry.source_url.as_str()).copied();
            let title = source_title(&entry.source_url, source, fetches.get(&entry.source_url));
            let read = tags::is_read(entry.mark, published, rebuilt_at);
            let facts = Facts {
                source_tags: source.map_or(&no_tags, |s| &s.tags),
                source_title: &title,
                published,
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

/// One configured source with what the database knows about it.
pub struct SourceStatus<'a> {
    pub source: &'a Source,
    pub title: String,
    pub entries: usize,
    /// The date of the newest stored entry.
    pub newest: Option<DateTime<Utc>>,
    /// The last sync; None if the source has never been synced.
    pub fetch: Option<Fetch>,
}

/// Every source in the config file, in file order.
pub fn sources<'a>(config: &'a Config, db: &Database) -> Result<Vec<SourceStatus<'a>>> {
    let mut fetches = db.fetches()?;
    let mut stats: HashMap<String, (usize, DateTime<Utc>)> = HashMap::new();
    for e in db.entries()? {
        let date = e.published_at.unwrap_or(e.first_seen_at);
        let (count, newest) = stats.entry(e.source_url).or_insert((0, date));
        *count += 1;
        *newest = (*newest).max(date);
    }
    Ok(config
        .sources
        .iter()
        .map(|source| {
            let fetch = fetches.remove(&source.url);
            let stat = stats.get(&source.url);
            SourceStatus {
                source,
                title: source_title(&source.url, Some(source), fetch.as_ref()),
                entries: stat.map_or(0, |s| s.0),
                newest: stat.map(|s| s.1),
                fetch,
            }
        })
        .collect())
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
        db.record_fetch_ok("https://misc.example/feed", Some("Misc Feed"), &Default::default(), now).unwrap();
        db.record_fetch_error("https://misc.example/feed", "timeout", now).unwrap();

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

        let status = sources(&cfg, &db).unwrap();
        assert_eq!(status.len(), 2); // only what the config lists; "gone" is not a source any more
        assert_eq!((status[0].title.as_str(), status[0].entries), ("AI Blog", 2));
        assert_eq!(status[0].newest, Some(now - Duration::days(1)));
        assert!(status[0].fetch.is_none());
        assert_eq!((status[1].title.as_str(), status[1].entries), ("Misc Feed", 1));
        assert_eq!(status[1].fetch.as_ref().unwrap().error.as_deref(), Some("timeout"));
    }

    #[test]
    fn undated_backlog_is_read_and_has_no_age() {
        let installed = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let cfg = config::parse("[[source]]\nurl = \"s\"\n").unwrap().config;
        let db = Database::open_in_memory(Some(installed)).unwrap();
        let add = |key: &str, seen: DateTime<Utc>| {
            let entry = NewEntry { source_url: "s", entry_key: key, url: None, title: key, published_at: None, word_count: None };
            db.insert_entry(&entry, seen).unwrap();
        };
        // The first sync brings the feed's undated backlog; a later sync brings one more entry.
        let first_sync = installed + Duration::minutes(5);
        let second_sync = installed + Duration::days(2);
        add("backlog-1", first_sync);
        add("backlog-2", first_sync);
        add("fresh", second_sync);

        let items = items(&cfg, &db, second_sync + Duration::hours(1)).unwrap();
        let get = |title: &str| items.iter().find(|i| i.entry.title == title).unwrap();
        for backlog in ["backlog-1", "backlog-2"] {
            assert!(get(backlog).read, "{backlog} should count as read");
            assert!(!get(backlog).tags.iter().any(|t| t.starts_with("age/")), "{backlog} has no known age");
        }
        assert!(!get("fresh").read);
        assert!(get("fresh").tags.contains("age/today"));
    }
}
