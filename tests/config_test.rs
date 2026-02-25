use std::io::Write;
use tempfile::NamedTempFile;
use tagrss::config::{load_folders, load_opml, load_rules};

#[test]
fn test_load_opml() {
    let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <body>
    <outline type="rss" text="Test Feed" xmlUrl="http://example.com/feed" tags="tech,news" />
    <outline type="rss" text="Another" xmlUrl="http://example.com/feed2" />
  </body>
</opml>"#;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(opml.as_bytes()).unwrap();

    let feeds = load_opml(file.path()).unwrap();
    assert_eq!(feeds.len(), 2);
    assert_eq!(feeds[0].title, "Test Feed");
    assert_eq!(feeds[0].xml_url, "http://example.com/feed");
    assert!(feeds[0].tags.contains("tech"));
    assert!(feeds[0].tags.contains("news"));
    assert!(feeds[1].tags.is_empty());
}

#[test]
fn test_load_opml_no_tags() {
    let opml = r#"<?xml version="1.0"?>
<opml version="2.0">
  <body>
    <outline type="rss" text="No Tags" xmlUrl="http://example.com/feed" />
  </body>
</opml>"#;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(opml.as_bytes()).unwrap();

    let feeds = load_opml(file.path()).unwrap();
    assert_eq!(feeds.len(), 1);
    assert!(feeds[0].tags.is_empty());
}

#[test]
fn test_load_rules() {
    let yaml = r#"
rules:
  - type: word_count
    min: 3000
    tag: long
  - type: contains
    pattern: "GPT"
    case_sensitive: false
    tag: tech/ai/llm
  - type: age
    max_days: 1
    tag: fresh
"#;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(yaml.as_bytes()).unwrap();

    let rules = load_rules(file.path()).unwrap();
    assert_eq!(rules.len(), 3);
}

#[test]
fn test_load_folders() {
    let yaml = r#"
folders:
  - name: Priority
    filter: important AND NOT long
  - name: AI News
    filter: tech/ai
"#;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(yaml.as_bytes()).unwrap();

    let folders = load_folders(file.path()).unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0].name, "Priority");
    assert_eq!(folders[1].name, "AI News");
}
