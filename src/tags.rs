//! Tags: hierarchical names, the reserved system namespaces, and computing an entry's tags.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashSet;

/// Namespaces owned by the system; user tags must not start with these.
pub const RESERVED: [&str; 4] = ["source", "age", "length", "state"];

/// Every concrete system tag except `source/<slug>`.
pub const SYSTEM_TAGS: [&str; 9] = [
    "age/today",
    "age/week",
    "age/month",
    "length/short",
    "length/medium",
    "length/long",
    "length/unknown",
    "state/read",
    "state/unread",
];

pub const SHORT_BELOW: u32 = 500;
pub const LONG_FROM: u32 = 3000;

/// True if `query` names `tag` or one of its ancestors: `tech` matches `tech/semi`, not `fintech`.
pub fn tag_matches(tag: &str, query: &str) -> bool {
    tag == query
        || (tag.len() > query.len()
            && tag.starts_with(query)
            && tag.as_bytes()[query.len()] == b'/')
}

pub fn has_tag(tags: &HashSet<String>, query: &str) -> bool {
    tags.iter().any(|t| tag_matches(t, query))
}

/// The namespace a tag belongs to if it is reserved, e.g. `Some("age")` for `age/today`.
pub fn reserved_namespace(tag: &str) -> Option<&'static str> {
    let first = tag.split('/').next().unwrap_or("");
    RESERVED.iter().copied().find(|ns| *ns == first)
}

/// Checks the shape of a tag: non-empty segments, no whitespace or parentheses.
pub fn validate(tag: &str) -> Result<(), String> {
    if tag.is_empty() {
        return Err("empty tag".into());
    }
    if tag.split('/').any(str::is_empty) {
        return Err(format!("tag \"{tag}\" has an empty segment"));
    }
    if tag.chars().any(|c| c.is_whitespace() || c == '(' || c == ')') {
        return Err(format!("tag \"{tag}\" contains whitespace or parentheses"));
    }
    Ok(())
}

/// Slug for `source/<slug>`: lowercase, letters and digits kept (any script), everything else becomes `-`.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The facts about one entry that its tags are computed from.
pub struct Facts<'a> {
    /// The union of the tags of every source the entry came from.
    pub source_tags: &'a HashSet<String>,
    /// The display title of every source the entry came from.
    pub source_titles: &'a [String],
    /// The entry's date: its earliest publish date, else when it was first seen.
    pub date: DateTime<Utc>,
    pub word_count: Option<u32>,
    pub read: bool,
}

/// An entry's tags: its sources' tags plus the system tags, as of `now`.
pub fn compute(facts: &Facts, now: DateTime<Utc>) -> HashSet<String> {
    let mut tags = facts.source_tags.clone();
    for title in facts.source_titles {
        tags.insert(format!("source/{}", slug(title)));
    }

    // A date in the future (clock skew, time zones) counts as just published.
    let age = (now - facts.date).max(Duration::zero());
    if age < Duration::days(1) {
        tags.insert("age/today".into());
    }
    if age < Duration::days(7) {
        tags.insert("age/week".into());
    }
    if age < Duration::days(30) {
        tags.insert("age/month".into());
    }

    let length = match facts.word_count {
        None => "length/unknown",
        Some(n) if n < SHORT_BELOW => "length/short",
        Some(n) if n < LONG_FROM => "length/medium",
        Some(_) => "length/long",
    };
    tags.insert(length.into());

    tags.insert(if facts.read { "state/read" } else { "state/unread" }.into());
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn set(tags: &[&str]) -> HashSet<String> {
        tags.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn hierarchical_matching_is_by_whole_segments() {
        assert!(tag_matches("tech/semi", "tech"));
        assert!(tag_matches("tech/semi", "tech/semi"));
        assert!(tag_matches("tech/semi/tsmc", "tech/semi"));
        assert!(!tag_matches("fintech", "tech"));
        assert!(!tag_matches("technology", "tech"));
        assert!(!tag_matches("tech", "tech/semi"));
        assert!(!tag_matches("topic/tech", "tech"));
        assert!(has_tag(&set(&["a", "topic/ai"]), "topic"));
    }

    #[test]
    fn reserved_and_validation() {
        assert_eq!(reserved_namespace("age/today"), Some("age"));
        assert_eq!(reserved_namespace("ageing"), None);
        assert!(validate("tech/semi").is_ok());
        assert!(validate("tech//semi").is_err());
        assert!(validate("tech/").is_err());
        assert!(validate("has space").is_err());
        assert!(validate("").is_err());
    }

    #[test]
    fn slugs_keep_any_script() {
        assert_eq!(slug("Lobsters graphics"), "lobsters-graphics");
        assert_eq!(slug("Two Minute Papers - YouTube"), "two-minute-papers-youtube");
        assert_eq!(slug("跟李沐学AI 的 bilibili 动态"), "跟李沐学ai-的-bilibili-动态");
        assert_eq!(slug("--- !!! ---"), "untitled");
    }

    fn tags_at(hours_ago: i64, words: Option<u32>, read: bool) -> HashSet<String> {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let none = HashSet::new();
        let titles = vec!["Feed".to_string()];
        let facts = Facts {
            source_tags: &none,
            source_titles: &titles,
            date: now - Duration::hours(hours_ago),
            word_count: words,
            read,
        };
        compute(&facts, now)
    }

    #[test]
    fn age_tags_overlap_and_clamp_future_dates() {
        let today = tags_at(3, None, false);
        assert!(today.contains("age/today") && today.contains("age/week") && today.contains("age/month"));
        let week = tags_at(72, None, false);
        assert!(!week.contains("age/today") && week.contains("age/week"));
        assert!(!tags_at(24 * 40, None, false).iter().any(|t| t.starts_with("age/")));
        assert!(tags_at(-48, None, false).contains("age/today"));
    }

    #[test]
    fn length_and_state_tags() {
        assert!(tags_at(1, None, false).contains("length/unknown"));
        assert!(tags_at(1, Some(0), false).contains("length/short"));
        assert!(tags_at(1, Some(499), false).contains("length/short"));
        assert!(tags_at(1, Some(500), false).contains("length/medium"));
        assert!(tags_at(1, Some(3000), false).contains("length/long"));
        assert!(tags_at(1, None, true).contains("state/read"));
        assert!(tags_at(1, None, false).contains("state/unread"));
    }

    #[test]
    fn every_source_contributes_tags_and_a_slug() {
        let now = Utc::now();
        let source_tags = set(&["topic/ai", "format/filter"]);
        let titles = vec!["Distill".to_string(), "Lobsters graphics".to_string()];
        let facts = Facts { source_tags: &source_tags, source_titles: &titles, date: now, word_count: None, read: false };
        let tags = compute(&facts, now);
        for t in ["topic/ai", "format/filter", "source/distill", "source/lobsters-graphics"] {
            assert!(tags.contains(t), "missing {t}");
        }
    }
}
