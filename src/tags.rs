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
    pub source_tags: &'a [String],
    pub source_title: &'a str,
    pub published_at: Option<DateTime<Utc>>,
    pub first_seen_at: DateTime<Utc>,
    pub word_count: Option<u32>,
    pub read: bool,
}

/// An entry's tags: its source's tags plus the system tags, as of `now`.
pub fn compute(facts: &Facts, now: DateTime<Utc>) -> HashSet<String> {
    let mut tags: HashSet<String> = facts.source_tags.iter().cloned().collect();
    tags.insert(format!("source/{}", slug(facts.source_title)));

    // A publish date in the future (clock skew, time zones) counts as just published.
    let when = facts.published_at.unwrap_or(facts.first_seen_at);
    let age = (now - when).max(Duration::zero());
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

/// Whether an entry counts as read: an explicit mark wins, otherwise anything from before a rebuild is read.
pub fn is_read(
    mark: Option<bool>,
    published_at: Option<DateTime<Utc>>,
    first_seen_at: DateTime<Utc>,
    rebuilt_at: Option<DateTime<Utc>>,
) -> bool {
    match mark {
        Some(read) => read,
        None => rebuilt_at.is_some_and(|r| published_at.unwrap_or(first_seen_at) < r),
    }
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

    fn facts(published_hours_ago: Option<i64>, words: Option<u32>, read: bool, now: DateTime<Utc>) -> Facts<'static> {
        Facts {
            source_tags: &[],
            source_title: "Feed",
            published_at: published_hours_ago.map(|h| now - Duration::hours(h)),
            first_seen_at: now - Duration::hours(2),
            word_count: words,
            read,
        }
    }

    #[test]
    fn age_tags_overlap_and_clamp_future_dates() {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let today = compute(&facts(Some(3), None, false, now), now);
        assert!(today.contains("age/today") && today.contains("age/week") && today.contains("age/month"));
        let week = compute(&facts(Some(72), None, false, now), now);
        assert!(!week.contains("age/today") && week.contains("age/week"));
        let old = compute(&facts(Some(24 * 40), None, false, now), now);
        assert!(!old.iter().any(|t| t.starts_with("age/")));
        let future = compute(&facts(Some(-48), None, false, now), now);
        assert!(future.contains("age/today"));
        // No publish date: first_seen_at (2 hours ago) is used.
        assert!(compute(&facts(None, None, false, now), now).contains("age/today"));
    }

    #[test]
    fn length_and_state_tags() {
        let now = Utc::now();
        let tags = |w, r| compute(&facts(Some(1), w, r, now), now);
        assert!(tags(None, false).contains("length/unknown"));
        assert!(tags(Some(0), false).contains("length/short"));
        assert!(tags(Some(499), false).contains("length/short"));
        assert!(tags(Some(500), false).contains("length/medium"));
        assert!(tags(Some(3000), false).contains("length/long"));
        assert!(tags(None, true).contains("state/read"));
        assert!(tags(None, false).contains("state/unread"));
    }

    #[test]
    fn source_tags_and_slug_are_included() {
        let now = Utc::now();
        let source_tags = vec!["topic/ai".to_string()];
        let f = Facts {
            source_tags: &source_tags,
            source_title: "Distill",
            published_at: None,
            first_seen_at: now,
            word_count: None,
            read: false,
        };
        let tags = compute(&f, now);
        assert!(tags.contains("topic/ai") && tags.contains("source/distill"));
    }

    #[test]
    fn explicit_marks_override_the_rebuild_rule() {
        let rebuilt = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let before = Some(rebuilt - Duration::days(1));
        let after = Some(rebuilt + Duration::days(1));
        let seen = rebuilt;
        assert!(is_read(None, before, seen, Some(rebuilt)));
        assert!(!is_read(None, after, seen, Some(rebuilt)));
        assert!(!is_read(Some(false), before, seen, Some(rebuilt)));
        assert!(is_read(Some(true), after, seen, Some(rebuilt)));
        assert!(!is_read(None, before, seen, None));
    }
}
