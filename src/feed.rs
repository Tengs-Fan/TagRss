//! Fetching and parsing feeds, and syncing their entries into the database.

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use reqwest::header::{HeaderName, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::StatusCode;
use std::time::Duration;

use crate::config::Config;
use crate::db::{Database, NewEntry, Validators};

const CONCURRENT_FETCHES: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How much of an entry's text stands in for a missing title.
const EXCERPT_CHARS: usize = 80;

/// What a feed said, reduced to the metadata TagRss keeps.
#[derive(Debug)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub entries: Vec<ParsedEntry>,
}

#[derive(Debug)]
pub struct ParsedEntry {
    pub key: String,
    pub url: Option<String>,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    pub word_count: Option<u32>,
}

pub fn parse(bytes: &[u8]) -> Result<ParsedFeed> {
    let feed = match feed_rs::parser::parse(bytes) {
        Ok(feed) => feed,
        // The usual reason: the feed is gone and the address now serves the site, a login page, or a parking page.
        Err(_) if looks_like_html(bytes) => bail!("not a feed: the address returns a web page"),
        Err(e) => return Err(e.into()),
    };
    let title = feed.title.map(|t| clean(&t.content)).filter(|t| !t.is_empty());
    let entries = feed
        .entries
        .into_iter()
        .map(|e| {
            let url = e.links.first().map(|l| l.href.clone()).filter(|u| !u.is_empty());
            // Only full content is counted. A summary is usually a teaser or a bare link
            // (Distill, aggregators), and counting it would tag long articles length/short.
            let text = e.content.and_then(|c| c.body).filter(|b| !b.trim().is_empty());
            let summary = e.summary.map(|s| s.content);
            // Microblog posts have no title; the start of their text is shown instead.
            let title = e
                .title
                .map(|t| clean(&t.content))
                .filter(|t| !t.is_empty())
                .or_else(|| text.as_deref().or(summary.as_deref()).and_then(excerpt))
                .unwrap_or_else(|| "Untitled".to_string());
            // feed-rs generates an id when the feed gives none; the link and title are a last resort.
            let key = Some(e.id)
                .filter(|id| !id.is_empty())
                .or_else(|| url.clone())
                .unwrap_or_else(|| title.clone());
            ParsedEntry {
                key,
                url,
                title,
                published_at: e.published.or(e.updated),
                word_count: text.map(|t| count_words(&t)),
            }
        })
        .collect();
    Ok(ParsedFeed { title, entries })
}

/// Decodes HTML entities (some feeds escape titles twice, leaving `&rsquo;` behind) and collapses whitespace.
fn clean(s: &str) -> String {
    html_escape::decode_html_entities(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first characters of an entry's text, without markup; None if there is no text.
fn excerpt(html: &str) -> Option<String> {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let text = clean(&text);
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(EXCERPT_CHARS) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text,
    })
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
    let head = head.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

/// Counts words in text or HTML. Han, Hiragana, and Katakana characters count one word each.
pub fn count_words(text: &str) -> u32 {
    let mut count = 0;
    let mut in_tag = false;
    let mut in_word = false;
    for c in text.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
            }
            continue;
        }
        if c == '<' {
            in_tag = true;
            in_word = false;
        } else if is_cjk(c) {
            count += 1;
            in_word = false;
        } else if c.is_alphanumeric() {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else if c.is_whitespace() {
            in_word = false;
        }
        // Other punctuation, such as an apostrophe or hyphen, continues the current word.
    }
    count
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}'       // Hiragana, Katakana
        | '\u{3400}'..='\u{4DBF}'     // CJK Extension A
        | '\u{4E00}'..='\u{9FFF}'     // CJK Unified Ideographs
        | '\u{F900}'..='\u{FAFF}'     // CJK Compatibility Ideographs
        | '\u{20000}'..='\u{2FA1F}'   // CJK Extensions B and later
    )
}

pub fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("tagrss/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// What a server answered when asked for a feed.
#[derive(Debug)]
pub enum Fetched {
    /// The feed has not changed since the response the validators came from.
    NotModified,
    Feed { feed: ParsedFeed, validators: Validators },
}

/// Fetches a feed. `validators` are those of the last successful fetch, so an unchanged feed is not sent again.
pub async fn fetch(client: &reqwest::Client, url: &str, validators: &Validators) -> Result<Fetched> {
    let mut request = client.get(url);
    if let Some(etag) = &validators.etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    if let Some(last_modified) = &validators.last_modified {
        request = request.header(IF_MODIFIED_SINCE, last_modified);
    }
    // Errors are shown next to the source they belong to, so the URL inside them is noise.
    let response = request.send().await.map_err(reqwest::Error::without_url)?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    let response = response.error_for_status().map_err(reqwest::Error::without_url)?;
    let header = |name: HeaderName| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let validators = Validators { etag: header(ETAG), last_modified: header(LAST_MODIFIED) };
    let bytes = response.bytes().await.map_err(reqwest::Error::without_url)?;
    Ok(Fetched::Feed { feed: parse(&bytes)?, validators })
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The feed was fetched; this many of its entries were new.
    New(usize),
    /// The server said the feed has not changed since the last fetch.
    Unchanged,
    Failed(String),
}

/// The outcome of syncing one source.
pub struct SyncResult {
    pub url: String,
    pub outcome: Outcome,
}

/// Fetches every configured source and stores new entries; one failing feed never stops the rest.
/// `progress` is called with (sources done, sources in total) as each one finishes.
/// Results are in the order of the config file.
pub async fn sync(
    config: &Config,
    db: &Database,
    now: DateTime<Utc>,
    mut progress: impl FnMut(usize, usize),
) -> Result<Vec<SyncResult>> {
    let client = client()?;
    let known = db.fetches()?;
    let mut fetches = stream::iter(config.sources.iter().enumerate())
        .map(|(index, source)| {
            let client = &client;
            let validators = known.get(&source.url).map(|f| f.validators.clone()).unwrap_or_default();
            async move { (index, source.url.as_str(), fetch(client, &source.url, &validators).await) }
        })
        .buffer_unordered(CONCURRENT_FETCHES);

    let mut results = Vec::new();
    while let Some((index, url, fetched)) = fetches.next().await {
        let outcome = match fetched {
            Ok(Fetched::Feed { feed, validators }) => Outcome::New(store(db, url, &feed, &validators, now)?),
            Ok(Fetched::NotModified) => {
                db.record_fetch_unchanged(url, now)?;
                Outcome::Unchanged
            }
            Err(e) => {
                let error = format!("{e:#}");
                db.record_fetch_error(url, &error, now)?;
                Outcome::Failed(error)
            }
        };
        results.push((index, SyncResult { url: url.to_string(), outcome }));
        progress(results.len(), config.sources.len());
    }
    results.sort_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
}

/// Stores a fetched feed's new entries and its validators together, so a feed is never
/// remembered as "seen up to here" without its entries.
fn store(db: &Database, url: &str, feed: &ParsedFeed, validators: &Validators, now: DateTime<Utc>) -> Result<usize> {
    db.transaction(|| {
        let mut added = 0;
        for e in &feed.entries {
            let entry = NewEntry {
                source_url: url,
                entry_key: &e.key,
                url: e.url.as_deref(),
                title: &e.title,
                published_at: e.published_at,
                word_count: e.word_count,
            };
            if db.insert_entry(&entry, now)? {
                added += 1;
            }
        }
        db.record_fetch_ok(url, feed.title.as_deref(), validators, now)?;
        Ok(added)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_plain_text_html_and_cjk() {
        assert_eq!(count_words("Hello world"), 2);
        assert_eq!(count_words("<p>Hello <b>big</b> world</p>"), 3);
        assert_eq!(count_words("Before the tag <br/>and after"), 5);
        assert_eq!(count_words("it's a well-known fact"), 4);
        assert_eq!(count_words("— … ·"), 0);
        assert_eq!(count_words("我们今天学习 Rust 语言"), 9);
        assert_eq!(count_words("ひらがなとカタカナ"), 9);
        assert_eq!(count_words(""), 0);
    }

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <title>  Example
     Blog </title>
  <item>
    <title>Plain description</title>
    <link>https://example.com/a</link>
    <guid>a-1</guid>
    <pubDate>Mon, 05 Oct 2026 10:00:00 GMT</pubDate>
    <description>One two three four</description>
  </item>
  <item>
    <title>No link at all</title>
    <guid>b-2</guid>
  </item>
</channel></rss>"#;

    #[test]
    fn parses_rss_without_dropping_entries() {
        let feed = parse(RSS.as_bytes()).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Example Blog"));
        assert_eq!(feed.entries.len(), 2);
        let a = &feed.entries[0];
        assert_eq!(a.key, "a-1");
        assert_eq!(a.url.as_deref(), Some("https://example.com/a"));
        // A description is a summary, not the article: its length is unknown.
        assert_eq!(a.word_count, None);
        assert!(a.published_at.is_some());
        let b = &feed.entries[1];
        assert_eq!(b.key, "b-2");
        assert_eq!(b.url, None);
        assert_eq!(b.word_count, None);
    }

    const YOUTUBE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/">
  <title>Some Channel</title>
  <entry>
    <id>yt:video:abc123</id>
    <title>A video</title>
    <link rel="alternate" href="https://www.youtube.com/watch?v=abc123"/>
    <published>2026-10-05T10:00:00+00:00</published>
    <updated>2026-10-06T10:00:00+00:00</updated>
    <media:group>
      <media:title>A video</media:title>
      <media:description>Long description that is not article text</media:description>
    </media:group>
  </entry>
</feed>"#;

    #[test]
    fn decodes_titles_escaped_twice() {
        let rss = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>T</title>
  <item><guid>1</guid><title>shoutout to &amp;ldquo;overmorrow&amp;rdquo; &amp;amp; I&amp;rsquo;m</title></item>
</channel></rss>"#;
        assert_eq!(parse(rss.as_bytes()).unwrap().entries[0].title, "shoutout to \u{201c}overmorrow\u{201d} & I\u{2019}m");
    }

    #[test]
    fn video_entries_have_unknown_length() {
        let feed = parse(YOUTUBE.as_bytes()).unwrap();
        let e = &feed.entries[0];
        assert_eq!(e.key, "yt:video:abc123");
        assert_eq!(e.word_count, None);
        assert_eq!(e.published_at.unwrap().to_rfc3339(), "2026-10-05T10:00:00+00:00");
    }

    #[test]
    fn entries_without_a_title_show_the_start_of_their_text() {
        let long = "word ".repeat(40);
        let atom = format!(
            r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>T</title>
  <entry><id>1</id><content type="html">&lt;p&gt;Just a &lt;b&gt;short&lt;/b&gt; note&lt;/p&gt;&lt;p&gt;Fish &amp;amp; chips.&lt;/p&gt;</content></entry>
  <entry><id>2</id><summary>{long}</summary></entry>
  <entry><id>3</id><content type="html">&lt;img src="a.png"/&gt;</content></entry>
  <entry><id>4</id><title>Has a title</title><summary>ignored</summary></entry>
</feed>"#
        );
        let entries = parse(atom.as_bytes()).unwrap().entries;
        assert_eq!(entries[0].title, "Just a short note Fish & chips.");
        assert_eq!(entries[1].title, format!("{}…", "word ".repeat(16).trim_end()));
        assert_eq!(entries[1].title.chars().count(), 80);
        assert_eq!(entries[2].title, "Untitled");
        assert_eq!(entries[3].title, "Has a title");
    }

    #[test]
    fn a_web_page_is_reported_as_not_a_feed() {
        let page = "\u{feff}\n  <!DOCTYPE html><html><head><title>Moved</title></head><body></body></html>";
        assert_eq!(parse(page.as_bytes()).unwrap_err().to_string(), "not a feed: the address returns a web page");
        // Anything else keeps the parser's own message.
        assert!(!parse(b"plain text").unwrap_err().to_string().contains("web page"));
    }

    #[test]
    fn rss_full_content_is_counted() {
        let rss = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel><title>T</title>
  <item><guid>1</guid><title>E</title>
    <description>A teaser</description>
    <content:encoded><![CDATA[<p>The full post has six words.</p>]]></content:encoded>
  </item>
</channel></rss>"#;
        assert_eq!(parse(rss.as_bytes()).unwrap().entries[0].word_count, Some(6));
    }

    #[test]
    fn atom_content_is_counted_not_summary() {
        let atom = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>T</title>
  <entry><id>x</id><title>E</title>
    <summary>short</summary>
    <content type="html">&lt;p&gt;one two three&lt;/p&gt;</content>
  </entry>
</feed>"#;
        assert_eq!(parse(atom.as_bytes()).unwrap().entries[0].word_count, Some(3));
    }
}
