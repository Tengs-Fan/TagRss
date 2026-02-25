use chrono::Utc;
use tagrss::models::{Article, Rule};

fn make_article(tags: &[&str]) -> Article {
    Article {
        id: 1,
        source_id: 1,
        url: "http://example.com".to_string(),
        title: "Test Article".to_string(),
        content: None,
        published_at: None,
        word_count: 100,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        read: false,
    }
}

fn make_article_with_content(tags: &[&str], title: &str, content: &str, word_count: u32) -> Article {
    Article {
        id: 1,
        source_id: 1,
        url: "http://example.com".to_string(),
        title: title.to_string(),
        content: Some(content.to_string()),
        published_at: Some(Utc::now()),
        word_count,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        read: false,
    }
}

// ==================== match_tag tests ====================

#[test]
fn test_match_tag_exact() {
    let article = make_article(&["tech/ai/llm", "important"]);
    assert!(article.match_tag("tech/ai/llm"));
    assert!(article.match_tag("important"));
}

#[test]
fn test_match_tag_hierarchical_prefix() {
    let article = make_article(&["tech/ai/llm"]);
    // Prefix matches
    assert!(article.match_tag("tech"));
    assert!(article.match_tag("tech/ai"));
    assert!(article.match_tag("tech/ai/llm"));
}

#[test]
fn test_match_tag_suffix_no_match() {
    let article = make_article(&["tech/ai/llm"]);
    // Suffix should NOT match
    assert!(!article.match_tag("llm"));
    assert!(!article.match_tag("ai/llm"));
}

#[test]
fn test_match_tag_middle_no_match() {
    let article = make_article(&["tech/ai/llm"]);
    // Middle segment should NOT match
    assert!(!article.match_tag("ai"));
}

#[test]
fn test_match_tag_longer_query_no_match() {
    let article = make_article(&["ai"]);
    // Query longer than tag should NOT match
    assert!(!article.match_tag("tech/ai"));
    assert!(!article.match_tag("ai/llm"));
}

#[test]
fn test_match_tag_partial_segment_no_match() {
    let article = make_article(&["technology/ai"]);
    // "tech" should NOT match "technology" (must be full segment)
    assert!(!article.match_tag("tech"));
}

#[test]
fn test_match_tag_multiple_tags() {
    let article = make_article(&["tech/ai/llm", "news/world", "important"]);
    assert!(article.match_tag("tech"));
    assert!(article.match_tag("news"));
    assert!(article.match_tag("news/world"));
    assert!(article.match_tag("important"));
    assert!(!article.match_tag("science"));
}

#[test]
fn test_match_tag_empty_tags() {
    let article = make_article(&[]);
    assert!(!article.match_tag("tech"));
    assert!(!article.match_tag(""));
}

// ==================== Rule::Contains tests ====================

#[test]
fn test_rule_contains_case_insensitive() {
    let article = make_article_with_content(&[], "GPT-4 is amazing", "This is about GPT", 100);
    let rule = Rule::Contains {
        pattern: "gpt".to_string(),
        case_sensitive: false,
        tag: "ai".to_string(),
    };
    assert_eq!(rule.apply(&article), Some("ai".to_string()));
}

#[test]
fn test_rule_contains_case_sensitive() {
    let article = make_article_with_content(&[], "GPT-4 is amazing", "", 100);
    let rule = Rule::Contains {
        pattern: "gpt".to_string(),
        case_sensitive: true,
        tag: "ai".to_string(),
    };
    assert_eq!(rule.apply(&article), None); // "gpt" != "GPT"
}

#[test]
fn test_rule_contains_in_content() {
    let article = make_article_with_content(&[], "Hello", "This mentions Rust programming", 100);
    let rule = Rule::Contains {
        pattern: "Rust".to_string(),
        case_sensitive: true,
        tag: "rust".to_string(),
    };
    assert_eq!(rule.apply(&article), Some("rust".to_string()));
}

#[test]
fn test_rule_contains_no_match() {
    let article = make_article_with_content(&[], "Hello World", "Nothing here", 100);
    let rule = Rule::Contains {
        pattern: "Python".to_string(),
        case_sensitive: false,
        tag: "python".to_string(),
    };
    assert_eq!(rule.apply(&article), None);
}

// ==================== Rule::WordCount tests ====================

#[test]
fn test_rule_word_count_min() {
    let article = make_article_with_content(&[], "Title", "Content", 5000);
    let rule = Rule::WordCount {
        min: Some(3000),
        max: None,
        tag: "long".to_string(),
    };
    assert_eq!(rule.apply(&article), Some("long".to_string()));
}

#[test]
fn test_rule_word_count_max() {
    let article = make_article_with_content(&[], "Title", "Content", 200);
    let rule = Rule::WordCount {
        min: None,
        max: Some(500),
        tag: "short".to_string(),
    };
    assert_eq!(rule.apply(&article), Some("short".to_string()));
}

#[test]
fn test_rule_word_count_range() {
    let article = make_article_with_content(&[], "Title", "Content", 1500);
    let rule = Rule::WordCount {
        min: Some(1000),
        max: Some(2000),
        tag: "medium".to_string(),
    };
    assert_eq!(rule.apply(&article), Some("medium".to_string()));
}

#[test]
fn test_rule_word_count_below_min() {
    let article = make_article_with_content(&[], "Title", "Content", 500);
    let rule = Rule::WordCount {
        min: Some(3000),
        max: None,
        tag: "long".to_string(),
    };
    assert_eq!(rule.apply(&article), None);
}

#[test]
fn test_rule_word_count_above_max() {
    let article = make_article_with_content(&[], "Title", "Content", 1000);
    let rule = Rule::WordCount {
        min: None,
        max: Some(500),
        tag: "short".to_string(),
    };
    assert_eq!(rule.apply(&article), None);
}

// ==================== Rule::Age tests ====================

#[test]
fn test_rule_age_fresh() {
    let mut article = make_article_with_content(&[], "Title", "Content", 100);
    article.published_at = Some(Utc::now()); // Published now
    let rule = Rule::Age {
        max_days: Some(1),
        min_days: None,
        tag: "fresh".to_string(),
    };
    assert_eq!(rule.apply(&article), Some("fresh".to_string()));
}

#[test]
fn test_rule_age_old() {
    let mut article = make_article_with_content(&[], "Title", "Content", 100);
    article.published_at = Some(Utc::now() - chrono::Duration::days(10));
    let rule = Rule::Age {
        max_days: None,
        min_days: Some(7),
        tag: "old".to_string(),
    };
    assert_eq!(rule.apply(&article), Some("old".to_string()));
}

#[test]
fn test_rule_age_no_published_date() {
    let mut article = make_article_with_content(&[], "Title", "Content", 100);
    article.published_at = None;
    let rule = Rule::Age {
        max_days: Some(1),
        min_days: None,
        tag: "fresh".to_string(),
    };
    assert_eq!(rule.apply(&article), None);
}

#[test]
fn test_rule_age_not_fresh_enough() {
    let mut article = make_article_with_content(&[], "Title", "Content", 100);
    article.published_at = Some(Utc::now() - chrono::Duration::days(5));
    let rule = Rule::Age {
        max_days: Some(1),
        min_days: None,
        tag: "fresh".to_string(),
    };
    assert_eq!(rule.apply(&article), None);
}
