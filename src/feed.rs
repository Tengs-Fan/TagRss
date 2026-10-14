//! Fetching and parsing feeds, and syncing their items into the database.

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

use crate::config::Config;
use crate::db::{Database, NewObservation};

const CONCURRENT_FETCHES: usize = 8;
/// Hosts rate-limit: 8 parallel requests to lobste.rs got 429 Too Many Requests.
const FETCHES_PER_HOST: usize = 2;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// What a feed said, reduced to the metadata TagRss keeps.
#[derive(Debug)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub items: Vec<ParsedItem>,
}

#[derive(Debug)]
pub struct ParsedItem {
    pub item_key: String,
    pub url: Option<String>,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    pub word_count: Option<u32>,
}

pub fn parse(bytes: &[u8]) -> Result<ParsedFeed> {
    let feed = feed_rs::parser::parse(bytes)?;
    let title = feed.title.map(|t| clean(&t.content)).filter(|t| !t.is_empty());
    let items = feed
        .entries
        .into_iter()
        .map(|e| {
            let url = e.links.first().map(|l| l.href.clone()).filter(|u| !u.is_empty());
            let title = e
                .title
                .map(|t| clean(&t.content))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "Untitled".to_string());
            // feed-rs generates an id when the feed gives none; the link and title are a last resort.
            let item_key = Some(e.id)
                .filter(|id| !id.is_empty())
                .or_else(|| url.clone())
                .unwrap_or_else(|| title.clone());
            // Only full content is counted. A summary is usually a teaser or a bare link
            // (Distill, aggregators), and counting it would tag long articles length/short.
            let text = e.content.and_then(|c| c.body).filter(|b| !b.trim().is_empty());
            ParsedItem {
                item_key,
                url,
                title,
                published_at: e.published.or(e.updated),
                word_count: text.map(|t| count_words(&t)),
            }
        })
        .collect();
    Ok(ParsedFeed { title, items })
}

/// Decodes HTML entities (some feeds escape titles twice, leaving `&rsquo;` behind) and collapses whitespace.
fn clean(s: &str) -> String {
    html_escape::decode_html_entities(s).split_whitespace().collect::<Vec<_>>().join(" ")
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
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("tagrss/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<ParsedFeed> {
    let bytes = client.get(url).send().await?.error_for_status()?.bytes().await?;
    parse(&bytes)
}

/// Progress of a sync, reported as each source finishes.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    Started { sources: usize },
    Fetched { url: String, new_observations: usize },
    Failed { url: String, error: String },
}

/// Fetches every configured source and stores new observations; one failing feed never stops the rest.
/// Only database errors are returned; feed errors are recorded on the source and reported as events.
pub async fn sync(config: &Config, db: &Database, now: DateTime<Utc>, on_event: &mut dyn FnMut(SyncEvent)) -> Result<()> {
    let client = client()?;
    on_event(SyncEvent::Started { sources: config.sources.len() });
    let urls: Vec<&str> = config.sources.iter().map(|s| s.url.as_str()).collect();
    let mut per_host: HashMap<String, Arc<Semaphore>> = HashMap::new();
    let mut fetches = stream::iter(interleave_by_host(&urls))
        .map(|url| {
            let client = &client;
            let host = per_host
                .entry(host_of(url))
                .or_insert_with(|| Arc::new(Semaphore::new(FETCHES_PER_HOST)))
                .clone();
            async move {
                let _permit = host.acquire().await;
                (url.to_string(), fetch(client, url).await)
            }
        })
        .buffer_unordered(CONCURRENT_FETCHES);

    while let Some((url, fetched)) = fetches.next().await {
        match fetched {
            Ok(feed) => {
                let new_observations = store(db, &url, &feed, now)?;
                on_event(SyncEvent::Fetched { url, new_observations });
            }
            Err(e) => {
                let error = format!("{e:#}");
                db.record_fetch(&url, None, now, Some(&error))?;
                on_event(SyncEvent::Failed { url, error });
            }
        }
    }
    Ok(())
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string())
}

/// Orders URLs round-robin across hosts, so feeds sharing a host (17 Lobsters tag feeds,
/// 131 YouTube channels) are spread out instead of queueing behind each other.
fn interleave_by_host<'a>(urls: &[&'a str]) -> Vec<&'a str> {
    let mut hosts: Vec<(String, std::collections::VecDeque<&'a str>)> = Vec::new();
    for url in urls {
        let host = host_of(url);
        match hosts.iter_mut().find(|(h, _)| *h == host) {
            Some((_, queue)) => queue.push_back(url),
            None => hosts.push((host, std::collections::VecDeque::from([*url]))),
        }
    }
    let mut out = Vec::with_capacity(urls.len());
    while out.len() < urls.len() {
        for (_, queue) in hosts.iter_mut() {
            if let Some(url) = queue.pop_front() {
                out.push(url);
            }
        }
    }
    out
}

fn store(db: &Database, url: &str, feed: &ParsedFeed, now: DateTime<Utc>) -> Result<usize> {
    let mut added = 0;
    for item in &feed.items {
        let observation = NewObservation {
            source_url: url,
            item_key: &item.item_key,
            url: item.url.as_deref(),
            title: &item.title,
            published_at: item.published_at,
            word_count: item.word_count,
        };
        if db.insert_observation(&observation, now)? {
            added += 1;
        }
    }
    db.record_fetch(url, feed.title.as_deref(), now, None)?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaves_sources_across_hosts() {
        let urls = ["https://a.com/1", "https://a.com/2", "https://a.com/3", "https://b.com/1", "https://c.com/1", "https://b.com/2"];
        assert_eq!(
            interleave_by_host(&urls),
            vec!["https://a.com/1", "https://b.com/1", "https://c.com/1", "https://a.com/2", "https://b.com/2", "https://a.com/3"]
        );
        assert!(interleave_by_host(&[]).is_empty());
    }

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
        assert_eq!(feed.items.len(), 2);
        let a = &feed.items[0];
        assert_eq!(a.item_key, "a-1");
        assert_eq!(a.url.as_deref(), Some("https://example.com/a"));
        // A description is a summary, not the article: its length is unknown.
        assert_eq!(a.word_count, None);
        assert!(a.published_at.is_some());
        let b = &feed.items[1];
        assert_eq!(b.item_key, "b-2");
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
        assert_eq!(parse(rss.as_bytes()).unwrap().items[0].title, "shoutout to \u{201c}overmorrow\u{201d} & I\u{2019}m");
    }

    #[test]
    fn video_entries_have_unknown_length() {
        let feed = parse(YOUTUBE.as_bytes()).unwrap();
        let e = &feed.items[0];
        assert_eq!(e.item_key, "yt:video:abc123");
        assert_eq!(e.word_count, None);
        assert_eq!(e.published_at.unwrap().to_rfc3339(), "2026-10-05T10:00:00+00:00");
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
        assert_eq!(parse(rss.as_bytes()).unwrap().items[0].word_count, Some(6));
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
        assert_eq!(parse(atom.as_bytes()).unwrap().items[0].word_count, Some(3));
    }
}
