//! End-to-end tests: the real binary, a config file, a database file, and a local feed server.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A feed server on localhost. `/feed.xml` serves an Atom feed with an ETag and honours
/// If-None-Match; `/page` serves a web page; everything else is a 404.
struct Server {
    base: String,
    /// Bumping this "publishes" a new entry and changes the ETag.
    version: Arc<AtomicUsize>,
    /// One line per request: "<path> <status>".
    log: Arc<Mutex<Vec<String>>>,
}

fn feed(version: usize) -> String {
    let mut entries = String::from(
        r#"<entry><id>future</id><title>From the future</title><link href="https://example.com/future"/><published>2099-01-01T00:00:00Z</published></entry>
<entry><id>old</id><title>Old news</title><link href="https://example.com/old"/><published>2001-01-01T00:00:00Z</published></entry>
<entry><id>undated</id><title>Undated backlog</title><link href="https://example.com/undated"/></entry>"#,
    );
    if version > 1 {
        entries.push_str(r#"<entry><id>fresh</id><title>Undated but fresh</title><link href="https://example.com/fresh"/></entry>"#);
    }
    format!(r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><title>Test Feed</title>{entries}</feed>"#)
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let version = Arc::new(AtomicUsize::new(1));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (v, l) = (version.clone(), log.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let request = String::from_utf8_lossy(&request).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                let etag = format!("\"v{}\"", v.load(Ordering::SeqCst));
                let if_none_match = request
                    .lines()
                    .find_map(|line| line.to_ascii_lowercase().starts_with("if-none-match:").then(|| line[14..].trim().to_string()));
                let (status, headers, body) = match path.as_str() {
                    "/feed.xml" if if_none_match.as_deref() == Some(etag.as_str()) => ("304 Not Modified", String::new(), String::new()),
                    "/feed.xml" => ("200 OK", format!("ETag: {etag}\r\n"), feed(v.load(Ordering::SeqCst))),
                    "/page" => ("200 OK", String::new(), "<!DOCTYPE html><html><body>Moved</body></html>".to_string()),
                    _ => ("404 Not Found", String::new(), String::new()),
                };
                l.lock().unwrap().push(format!("{path} {}", &status[..3]));
                let response = format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Server { base, version, log }
    }

    fn requests(&self) -> Vec<String> {
        let mut log = self.log.lock().unwrap().clone();
        log.sort();
        log
    }
}

/// A config file and database in a temporary directory, and a way to run the binary on them.
struct Setup {
    dir: tempfile::TempDir,
}

struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Setup {
    fn new(config: &str) -> Setup {
        let setup = Setup { dir: tempfile::tempdir().unwrap() };
        std::fs::write(setup.config(), config).unwrap();
        setup
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("tagrss.toml")
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tagrss"));
        command.arg("--config").arg(self.config()).arg("--db").arg(self.dir.path().join("data/tagrss.db")).args(args);
        // The feed server is on localhost; a proxy from the environment must not get in the way.
        for var in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
            command.env_remove(var);
        }
        let out = command.output().unwrap();
        Output {
            ok: out.status.success(),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }

    /// Runs a command that must succeed and returns its stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(out.ok, "{args:?} failed: {}", out.stderr);
        out.stdout
    }
}

/// The "Unread" and "Total" columns of one folder in the output of `folders`.
fn counts(folders: &str, name: &str) -> (usize, usize) {
    let line = folders.lines().find(|l| l.starts_with(name)).unwrap_or_else(|| panic!("no folder {name} in:\n{folders}"));
    let mut numbers = line[name.len()..].split_whitespace().map(|n| n.parse().unwrap());
    (numbers.next().unwrap(), numbers.next().unwrap())
}

#[test]
fn a_reading_session_from_first_sync_to_marks() {
    let server = Server::start();
    let base = &server.base;
    let setup = Setup::new(&format!(
        r#"
        [[source]]
        url = "{base}/feed.xml"
        tags = ["topic/test"]

        [[source]]
        url = "{base}/page"
        title = "Moved away"

        [[source]]
        url = "{base}/gone"

        [[folder]]
        name = "Unread"
        rule = "state/unread"

        [[folder]]
        name = "Test"
        rule = "topic/test"

        [[folder]]
        name = "Today"
        rule = "age/today"
        "#
    ));

    assert!(setup.ok(&["check"]).contains("3 sources, 3 folders"));
    assert!(setup.ok(&["sources"]).contains("3 sources: 0 ok, 0 failed, 3 never synced"));

    // First sync: the backlog arrives, and the two broken sources are named with a reason.
    let sync = setup.run(&["sync"]);
    assert!(sync.ok);
    assert_eq!(sync.stdout, "3 new entries from 3 sources (0 unchanged, 2 failed)\n");
    assert!(sync.stderr.contains("failed  Moved away: not a feed: the address returns a web page"), "{}", sync.stderr);
    assert!(sync.stderr.contains("/gone: HTTP status client error (404 Not Found)"), "{}", sync.stderr);

    // Inbox zero: of the backlog, only the entry dated after installation is unread.
    // The undated one is not "published today" just because it was fetched today.
    let folders = setup.ok(&["folders"]);
    assert_eq!(counts(&folders, "Unread"), (1, 1), "{folders}");
    assert_eq!(counts(&folders, "Test"), (1, 3), "{folders}");
    assert_eq!(counts(&folders, "Today"), (1, 1), "{folders}");
    let list = setup.ok(&["list", "test"]);
    let lines: Vec<&str> = list.lines().collect();
    assert_eq!(lines.len(), 3, "{list}");
    assert!(lines[0].starts_with('*') && lines[0].contains("Test Feed") && lines[0].ends_with("From the future"), "{list}");
    assert!(lines.iter().skip(1).all(|l| l.starts_with(' ')), "{list}");

    // Second sync: the feed is unchanged, and the server was asked conditionally.
    assert_eq!(setup.ok(&["sync"]), "0 new entries from 3 sources (1 unchanged, 2 failed)\n");
    assert_eq!(server.requests(), ["/feed.xml 200", "/feed.xml 304", "/gone 404", "/gone 404", "/page 200", "/page 200"]);

    // The feed publishes an undated entry. Seen between two syncs, it is new: unread, and from today.
    server.version.store(2, Ordering::SeqCst);
    assert_eq!(setup.ok(&["sync"]), "1 new entry from 3 sources (0 unchanged, 2 failed)\n");
    let folders = setup.ok(&["folders"]);
    assert_eq!(counts(&folders, "Unread"), (2, 2), "{folders}");
    assert_eq!(counts(&folders, "Today"), (2, 2), "{folders}");

    // Marks: a whole folder, then single entries by id.
    assert_eq!(setup.ok(&["read", "--folder", "Unread"]), "Marked 2 entries read in \"Unread\"\n");
    assert_eq!(counts(&setup.ok(&["folders"]), "Unread"), (0, 0));
    assert_eq!(setup.ok(&["read", "--folder", "Unread"]), "Marked 0 entries read in \"Unread\"\n");
    let list = setup.ok(&["list", "Test"]);
    let id_of = |title: &str| {
        let line = list.lines().find(|l| l.ends_with(title)).unwrap_or_else(|| panic!("no {title} in:\n{list}"));
        line.trim_start_matches('*').split_whitespace().next().unwrap().to_string()
    };
    setup.ok(&["unread", &id_of("Old news"), &id_of("Undated backlog")]);
    assert_eq!(counts(&setup.ok(&["folders"]), "Unread"), (2, 2));
    setup.ok(&["read", &id_of("Old news")]);
    assert_eq!(counts(&setup.ok(&["folders"]), "Unread"), (1, 1));
    let missing = setup.run(&["read", "424242"]);
    assert!(!missing.ok);
    assert_eq!(missing.stderr, "error: no entry 424242\n");

    // Sources: the tag to use in rules, and what is wrong with the broken ones.
    let sources = setup.ok(&["sources"]);
    assert!(sources.contains("source/test-feed"), "{sources}");
    assert!(sources.contains("source/moved-away"), "{sources}");
    assert!(sources.ends_with("3 sources: 1 ok, 2 failed, 0 never synced\n"), "{sources}");
    let failed = setup.ok(&["sources", "--failed"]);
    assert!(!failed.contains("source/test-feed"), "{failed}");
    assert!(failed.contains(&format!("{base}/page")) && failed.contains("not a feed"), "{failed}");
}

#[test]
fn import_creates_the_config_and_export_gives_it_back() {
    let setup = Setup { dir: tempfile::tempdir().unwrap() };
    let missing = setup.run(&["folders"]);
    assert!(!missing.ok);
    assert!(missing.stderr.starts_with("error: no config file at "), "{}", missing.stderr);

    let opml = setup.dir.path().join("feeds.opml");
    std::fs::write(
        &opml,
        r#"<opml version="2.0"><body>
             <outline type="rss" text="One" xmlUrl="https://one.example/feed" tags="topic/ai" />
             <outline type="rss" text="Two" xmlUrl="https://two.example/feed" />
           </body></opml>"#,
    )
    .unwrap();
    let imported = setup.ok(&["import", opml.to_str().unwrap()]);
    assert!(imported.starts_with("Added 2 sources to "), "{imported}");
    assert!(setup.ok(&["import", opml.to_str().unwrap()]).starts_with("Added 0 sources to "));
    assert!(setup.ok(&["check"]).contains("2 sources, 0 folders"));

    let exported = setup.ok(&["export"]);
    assert!(exported.contains(r#"text="One" title="One" xmlUrl="https://one.example/feed" tags="topic/ai""#), "{exported}");
}

#[test]
fn config_errors_name_every_offending_entry() {
    let setup = Setup::new(
        r#"
        [[source]]
        url = "https://a.example/feed"
        tags = ["state/done"]

        [[folder]]
        name = "Broken"
        rule = "topic AND"
        "#,
    );
    let out = setup.run(&["folders"]);
    assert!(!out.ok);
    let lines: Vec<&str> = out.stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{}", out.stderr);
    assert!(lines[0].starts_with("error: invalid config file "));
    assert_eq!(lines[1], r#"  source #1 (https://a.example/feed): tag "state/done" uses the reserved prefix "state/""#);
    assert_eq!(lines[2], r#"  folder #1 (Broken): rule "topic AND": rule ends where a tag was expected"#);
}
