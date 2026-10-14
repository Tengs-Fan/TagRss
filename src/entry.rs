//! Entries: articles computed from observations, grouped by normalized link.

use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};

use crate::config::{Config, Source};
use crate::db::{Fetch, Observation};
use crate::tags::{self, Facts};

/// An article as the user sees it.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The smallest id among its observations; never changes as observations are added.
    pub id: i64,
    pub observation_ids: Vec<i64>,
    pub title: String,
    pub url: Option<String>,
    /// The earliest publish date, else the earliest first-seen time.
    pub date: DateTime<Utc>,
    pub word_count: Option<u32>,
    pub read: bool,
    /// Display titles of the sources it came from.
    pub sources: Vec<String>,
    pub tags: HashSet<String>,
}

const TRACKING_PARAMS: [&str; 7] = ["ref", "ref_src", "source", "fbclid", "gclid", "mc_cid", "mc_eid"];

/// The link used to decide that two observations are the same article.
pub fn normalize_url(raw: &str) -> String {
    let raw = raw.trim();
    let Ok(mut url) = url::Url::parse(raw) else {
        return raw.to_string();
    };
    if url.scheme() == "http" {
        let _ = url.set_scheme("https");
    }
    if let Some(host) = url.host_str() {
        if let Some(bare) = host.strip_prefix("www.") {
            let bare = bare.to_string();
            let _ = url.set_host(Some(&bare));
        }
    }
    url.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && !TRACKING_PARAMS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
    let path = url.path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        url.set_path(path.trim_end_matches('/'));
    }
    url.to_string()
}

/// Groups observations into articles. A normalized link groups observations only if
/// no single source repeats it; otherwise, and when there is no link, each stands alone.
pub fn group(observations: Vec<Observation>) -> Vec<Vec<Observation>> {
    let mut by_link: HashMap<String, Vec<Observation>> = HashMap::new();
    let mut groups = Vec::new();
    for o in observations {
        match o.url.as_deref().map(normalize_url) {
            Some(key) => by_link.entry(key).or_default().push(o),
            None => groups.push(vec![o]),
        }
    }
    for (_, members) in by_link {
        let mut sources = HashSet::new();
        let reliable = members.iter().all(|o| sources.insert(o.source_url.as_str()));
        if reliable {
            groups.push(members);
        } else {
            groups.extend(members.into_iter().map(|o| vec![o]));
        }
    }
    groups
}

/// Builds every entry with its tags as of `now`, newest first.
pub fn build(config: &Config, observations: Vec<Observation>, fetches: &HashMap<String, Fetch>, now: DateTime<Utc>) -> Vec<Entry> {
    let sources: HashMap<&str, &Source> = config.sources.iter().map(|s| (s.url.as_str(), s)).collect();
    let mut entries: Vec<Entry> = group(observations)
        .into_iter()
        .map(|mut members| {
            members.sort_by_key(|o| o.id);
            merge(&members, &sources, fetches, now)
        })
        .collect();
    entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
    entries
}

/// Merges one article's observations; `members` is sorted by id.
fn merge(members: &[Observation], sources: &HashMap<&str, &Source>, fetches: &HashMap<String, Fetch>, now: DateTime<Utc>) -> Entry {
    // The earliest publish date wins; observations without one count only if none has one.
    let dated = members.iter().filter_map(|o| o.published_at.map(|d| (d, o))).min_by_key(|(d, o)| (*d, o.id));
    let (date, primary) = dated.unwrap_or_else(|| {
        let o = members.iter().min_by_key(|o| (o.first_seen_at, o.id)).expect("a group is never empty");
        (o.first_seen_at, o)
    });

    let read = members
        .iter()
        .filter_map(|o| o.mark.as_ref())
        .max_by_key(|m| m.marked_at)
        .is_some_and(|m| m.read);

    let mut source_tags = HashSet::new();
    let mut titles: Vec<String> = Vec::new();
    for o in members {
        let source = sources.get(o.source_url.as_str()).copied();
        if let Some(s) = source {
            source_tags.extend(s.tags.iter().cloned());
        }
        let title = source_title(&o.source_url, source, fetches.get(&o.source_url));
        if !titles.contains(&title) {
            titles.push(title);
        }
    }

    let word_count = members.iter().filter_map(|o| o.word_count).max();
    let facts = Facts { source_tags: &source_tags, source_titles: &titles, date, word_count, read };
    Entry {
        id: members[0].id,
        observation_ids: members.iter().map(|o| o.id).collect(),
        title: primary.title.clone(),
        url: primary.url.clone(),
        date,
        word_count,
        read,
        tags: tags::compute(&facts, now),
        sources: titles,
    }
}

/// A source's display title: the config's title, else the feed's own, else its URL.
pub fn source_title(url: &str, source: Option<&Source>, fetch: Option<&Fetch>) -> String {
    source
        .and_then(|s| s.title.clone())
        .or_else(|| fetch.and_then(|f| f.feed_title.clone()))
        .unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::db::Mark;
    use chrono::{Duration, TimeZone};

    #[test]
    fn normalizes_links() {
        let n = normalize_url;
        assert_eq!(n("http://www.Example.com/post/"), "https://example.com/post");
        assert_eq!(n("https://example.com/post#comments"), "https://example.com/post");
        assert_eq!(n("https://example.com/p?utm_source=rss&utm_medium=feed&id=7&ref=hn"), "https://example.com/p?id=7");
        assert_eq!(n("https://example.com/"), "https://example.com/");
        // Meaningful query parameters stay: two videos are two articles.
        assert_ne!(n("https://www.youtube.com/watch?v=a"), n("https://www.youtube.com/watch?v=b"));
        assert_eq!(n("not a url"), "not a url");
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 10, 12, 0, 0).unwrap()
    }

    fn obs(id: i64, source: &str, url: Option<&str>, title: &str, days_ago: Option<i64>, words: Option<u32>) -> Observation {
        Observation {
            id,
            source_url: source.to_string(),
            url: url.map(str::to_string),
            title: title.to_string(),
            published_at: days_ago.map(|d| now() - Duration::days(d)),
            first_seen_at: now(),
            word_count: words,
            mark: None,
        }
    }

    fn config() -> Config {
        config::parse(
            r#"
            [[source]]
            url = "blog"
            title = "The Blog"
            tags = ["topic/os"]

            [[source]]
            url = "lobsters"
            title = "Lobsters"
            tags = ["format/filter"]
        "#,
        )
        .unwrap()
        .config
    }

    #[test]
    fn one_article_from_two_sources_is_one_entry() {
        let observations = vec![
            // Lobsters is observed first, with a later (submission) date and only a teaser.
            obs(5, "lobsters", Some("https://blog.example/post?utm_source=lobsters"), "Lobsters title", Some(1), None),
            obs(9, "blog", Some("http://www.blog.example/post/"), "The real title", Some(3), Some(4200)),
        ];
        let entries = build(&config(), observations, &HashMap::new(), now());
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.id, 5, "the smallest observation id is stable");
        assert_eq!(e.observation_ids, vec![5, 9]);
        assert_eq!(e.title, "The real title", "title comes with the earliest date");
        assert_eq!(e.date, now() - Duration::days(3));
        assert_eq!(e.word_count, Some(4200));
        for t in ["topic/os", "format/filter", "source/the-blog", "source/lobsters", "length/long", "age/week"] {
            assert!(e.tags.contains(t), "missing {t}");
        }
    }

    #[test]
    fn a_link_repeated_within_one_source_groups_nothing() {
        let home = Some("https://podcast.example/");
        let observations = vec![
            obs(1, "blog", home, "Episode 1", Some(9), None),
            obs(2, "blog", home, "Episode 2", Some(2), None),
            obs(3, "lobsters", home, "Linked", Some(1), None),
            obs(4, "blog", None, "No link", Some(1), None),
            obs(5, "lobsters", None, "No link either", Some(1), None),
        ];
        assert_eq!(build(&config(), observations, &HashMap::new(), now()).len(), 5);
    }

    #[test]
    fn the_most_recent_mark_on_any_observation_wins() {
        let mark = |read, minutes| Some(Mark { read, marked_at: now() + Duration::minutes(minutes) });
        let mut a = obs(1, "blog", Some("https://x.example/a"), "A", Some(1), None);
        let mut b = obs(2, "lobsters", Some("https://x.example/a"), "A", Some(1), None);
        a.mark = mark(true, 1);
        b.mark = mark(false, 2);
        let entries = build(&config(), vec![a.clone(), b.clone()], &HashMap::new(), now());
        assert!(!entries[0].read);
        b.mark = None;
        assert!(build(&config(), vec![a, b], &HashMap::new(), now())[0].read);
    }

    #[test]
    fn entries_without_dates_use_first_seen_and_sort_newest_first() {
        let mut old = obs(1, "blog", Some("https://x.example/old"), "Old", Some(5), None);
        let mut undated = obs(2, "blog", Some("https://x.example/undated"), "Undated", None, None);
        undated.first_seen_at = now() - Duration::days(1);
        old.first_seen_at = now();
        let entries = build(&config(), vec![old, undated], &HashMap::new(), now());
        assert_eq!(entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), vec!["Undated", "Old"]);
    }

    #[test]
    fn sources_removed_from_the_config_keep_their_entries() {
        let entries = build(&config(), vec![obs(1, "gone", Some("https://g.example/x"), "Gone", Some(1), None)], &HashMap::new(), now());
        assert_eq!(entries[0].sources, vec!["gone"]);
        assert!(!entries[0].tags.iter().any(|t| t.starts_with("topic/") || t.starts_with("format/")));
    }
}
