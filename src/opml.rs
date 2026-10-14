//! OPML import and export, for moving sources between TagRss and other readers.
//! OPML is never a source of truth: import appends to the config file, export reads from it.

use anyhow::{bail, Result};
use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashSet;
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

use crate::config::Config;
use crate::tags;

#[derive(Debug, PartialEq)]
pub struct OpmlFeed {
    pub title: Option<String>,
    pub url: String,
    pub tags: Vec<String>,
}

/// Reads every `<outline>` with an `xmlUrl`; a comma-separated `tags` attribute is kept.
pub fn parse(opml: &str) -> Result<Vec<OpmlFeed>> {
    let mut reader = Reader::from_str(opml);
    reader.config_mut().trim_text(true);
    let mut feeds = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"outline" => {
                if let Some(feed) = outline(&e)? {
                    feeds.push(feed);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(feeds)
}

fn outline(e: &BytesStart) -> Result<Option<OpmlFeed>> {
    let (mut url, mut text, mut title, mut tags) = (None, None, None, Vec::new());
    for attr in e.attributes() {
        let attr = attr?;
        let v = attr.unescape_value()?.trim().to_string();
        match attr.key.as_ref() {
            b"xmlUrl" => url = Some(v),
            b"text" => text = Some(v),
            b"title" => title = Some(v),
            b"tags" => tags = v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
            _ => {}
        }
    }
    Ok(url.filter(|u| !u.is_empty()).map(|url| OpmlFeed {
        title: text.or(title).filter(|t| !t.is_empty()),
        url,
        tags,
    }))
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub added: usize,
    pub already_present: usize,
    /// Tags left out because they are malformed or use a reserved prefix.
    pub dropped_tags: Vec<String>,
}

/// Appends every OPML feed not already in the config as a `[[source]]`, keeping the file's comments and layout.
pub fn import(config_text: &str, opml: &str) -> Result<(String, ImportReport)> {
    let mut doc: DocumentMut = config_text.parse()?;
    let sources = doc.entry("source").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let Some(sources) = sources.as_array_of_tables_mut() else {
        bail!("\"source\" in the config file is not a list of [[source]] tables");
    };
    let mut present: HashSet<String> = sources
        .iter()
        .filter_map(|t| t.get("url").and_then(|u| u.as_str()).map(str::to_string))
        .collect();

    let mut report = ImportReport::default();
    for feed in parse(opml)? {
        if !present.insert(feed.url.clone()) {
            report.already_present += 1;
            continue;
        }
        let mut table = Table::new();
        table["url"] = value(&feed.url);
        if let Some(title) = &feed.title {
            table["title"] = value(title);
        }
        let mut kept = Array::new();
        for tag in feed.tags {
            if tags::validate(&tag).is_ok() && tags::reserved_namespace(&tag).is_none() {
                kept.push(tag);
            } else {
                report.dropped_tags.push(format!("{}: {tag}", feed.url));
            }
        }
        if !kept.is_empty() {
            table["tags"] = value(kept);
        }
        sources.push(table);
        report.added += 1;
    }
    Ok((doc.to_string(), report))
}

pub fn export(config: &Config) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head>\n    <title>TagRss</title>\n  </head>\n  <body>\n");
    for s in &config.sources {
        let title = escape(s.title.as_deref().unwrap_or(&s.url)).into_owned();
        out.push_str(&format!(
            "    <outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{}\"",
            escape(&s.url)
        ));
        if !s.tags.is_empty() {
            out.push_str(&format!(" tags=\"{}\"", escape(s.tags.join(","))));
        }
        out.push_str(" />\n");
    }
    out.push_str("  </body>\n</opml>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    const OPML: &str = r#"<?xml version="1.0"?>
<opml version="2.0"><body>
  <outline text="Group">
    <outline type="rss" text="A &amp; B" xmlUrl="https://a.example/feed?x=1&amp;y=2" tags="topic/ai, depth/deep" />
  </outline>
  <outline type="rss" text="Bad tags" xmlUrl="https://b.example/feed" tags="age/new,ok,bad//x" />
  <outline type="rss" xmlUrl="https://c.example/feed" />
  <outline text="Not a feed" />
</body></opml>"#;

    #[test]
    fn parses_nested_outlines_and_unescapes() {
        let feeds = parse(OPML).unwrap();
        assert_eq!(feeds.len(), 3);
        assert_eq!(feeds[0].url, "https://a.example/feed?x=1&y=2");
        assert_eq!(feeds[0].title.as_deref(), Some("A & B"));
        assert_eq!(feeds[0].tags, vec!["topic/ai", "depth/deep"]);
        assert_eq!(feeds[2].title, None);
    }

    #[test]
    fn import_appends_keeps_comments_and_skips_known_feeds() {
        let existing = "# my sources\n[[source]]\nurl = \"https://c.example/feed\" # keep me\n\n[[folder]]\nname = \"All\"\nrule = \"state/unread\"\n";
        let (text, report) = import(existing, OPML).unwrap();
        assert_eq!(report.added, 2);
        assert_eq!(report.already_present, 1);
        assert_eq!(report.dropped_tags.len(), 2, "{:?}", report.dropped_tags);
        assert!(text.contains("# my sources") && text.contains("# keep me"), "{text}");

        let loaded = config::parse(&text).unwrap();
        assert_eq!(loaded.config.sources.len(), 3);
        assert_eq!(loaded.config.folders.len(), 1);
        let b = loaded.config.sources.iter().find(|s| s.url == "https://b.example/feed").unwrap();
        assert_eq!(b.tags, vec!["ok"]);

        // Importing again adds nothing.
        let (_, again) = import(&text, OPML).unwrap();
        assert_eq!(again.added, 0);
    }

    #[test]
    fn import_into_an_empty_file() {
        let (text, report) = import("", OPML).unwrap();
        assert_eq!(report.added, 3);
        assert_eq!(config::parse(&text).unwrap().config.sources.len(), 3);
    }

    #[test]
    fn export_round_trips_through_import() {
        let cfg = config::parse(&import("", OPML).unwrap().0).unwrap().config;
        let back = parse(&export(&cfg)).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].url, "https://a.example/feed?x=1&y=2");
        assert_eq!(back[0].tags, vec!["topic/ai", "depth/deep"]);
    }
}
