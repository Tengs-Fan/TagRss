//! The core: the one interface every user interface uses. It never prints and never launches anything.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::{self, Config, Folder};
use crate::db::Database;
use crate::entry::{self, Entry};
use crate::feed::{self, SyncEvent};
use crate::opml::{self, ImportReport};

pub struct Core {
    config_path: PathBuf,
    db_path: PathBuf,
    config: Config,
    db: Database,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FolderSummary {
    pub name: String,
    pub unread: usize,
    pub total: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceStatus {
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
    pub fetched_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SyncSummary {
    pub sources: usize,
    pub new_observations: usize,
    pub failed: usize,
}

/// Everything a sync needs, detached from the `Core` so it can run on another thread
/// with its own database connection while a user interface keeps reading.
pub struct Syncer {
    config: Config,
    db_path: PathBuf,
}

impl Syncer {
    pub fn run(self, mut on_event: impl FnMut(&SyncEvent)) -> Result<SyncSummary> {
        let db = Database::open(&self.db_path)?;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let mut summary = SyncSummary::default();
        runtime.block_on(feed::sync(&self.config, &db, Utc::now(), &mut |event| {
            match &event {
                SyncEvent::Started { sources } => summary.sources = *sources,
                SyncEvent::Fetched { new_observations, .. } => summary.new_observations += new_observations,
                SyncEvent::Failed { .. } => summary.failed += 1,
            }
            on_event(&event);
        }))?;
        Ok(summary)
    }
}

impl Core {
    /// Opens the config and the database; an invalid config is an error listing every problem.
    pub fn open(config_path: &Path, db_path: &Path) -> Result<(Core, Vec<String>)> {
        let loaded = config::load(config_path)?;
        let db = Database::open(db_path)?;
        let core = Core { config_path: config_path.to_path_buf(), db_path: db_path.to_path_buf(), config: loaded.config, db };
        Ok((core, loaded.warnings))
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Rereads the config file. If the new file is invalid, the old config stays in use.
    pub fn reload_config(&mut self) -> Result<Vec<String>> {
        let loaded = config::load(&self.config_path)?;
        self.config = loaded.config;
        Ok(loaded.warnings)
    }

    fn all_entries(&self) -> Result<Vec<Entry>> {
        let observations = self.db.observations()?;
        let fetches = self.db.fetches()?;
        Ok(entry::build(&self.config, observations, &fetches, Utc::now()))
    }

    /// Finds a folder by exact name, else by a unique case-insensitive match.
    pub fn folder(&self, name: &str) -> Result<&Folder> {
        if let Some(f) = self.config.folders.iter().find(|f| f.name == name) {
            return Ok(f);
        }
        let lower = name.to_lowercase();
        let close: Vec<&Folder> = self.config.folders.iter().filter(|f| f.name.to_lowercase() == lower).collect();
        if let [only] = close.as_slice() {
            return Ok(only);
        }
        let names: Vec<&str> = self.config.folders.iter().map(|f| f.name.as_str()).collect();
        bail!("no folder named \"{name}\"; folders are: {}", names.join(", "))
    }

    pub fn folders(&self) -> Result<Vec<FolderSummary>> {
        let entries = self.all_entries()?;
        Ok(self
            .config
            .folders
            .iter()
            .map(|f| {
                let (mut unread, mut total) = (0, 0);
                for e in entries.iter().filter(|e| f.rule.matches(&e.tags)) {
                    total += 1;
                    unread += usize::from(!e.read);
                }
                FolderSummary { name: f.name.clone(), unread, total }
            })
            .collect())
    }

    /// A folder's entries, newest first.
    pub fn entries(&self, folder: &str) -> Result<Vec<Entry>> {
        let folder = self.folder(folder)?;
        Ok(self.all_entries()?.into_iter().filter(|e| folder.rule.matches(&e.tags)).collect())
    }

    /// Resolves ids of any observations to their entries.
    fn resolve(&self, ids: &[i64]) -> Result<Vec<Entry>> {
        let entries = self.all_entries()?;
        let by_observation: HashMap<i64, &Entry> =
            entries.iter().flat_map(|e| e.observation_ids.iter().map(move |id| (*id, e))).collect();
        let mut found = Vec::new();
        let mut missing = Vec::new();
        for id in ids {
            match by_observation.get(id) {
                Some(e) => found.push((*e).clone()),
                None => missing.push(id.to_string()),
            }
        }
        if !missing.is_empty() {
            bail!("no entry {}", missing.join(", "));
        }
        Ok(found)
    }

    /// Marks entries read or unread; any of an entry's observation ids identifies it.
    pub fn mark(&mut self, ids: &[i64], read: bool) -> Result<()> {
        let observation_ids: Vec<i64> = self.resolve(ids)?.into_iter().flat_map(|e| e.observation_ids).collect();
        self.db.set_marks(&observation_ids, read, Utc::now())
    }

    /// Marks everything the folder contains right now read; returns how many entries changed.
    pub fn mark_folder_read(&mut self, folder: &str) -> Result<usize> {
        let unread: Vec<Entry> = self.entries(folder)?.into_iter().filter(|e| !e.read).collect();
        let observation_ids: Vec<i64> = unread.iter().flat_map(|e| e.observation_ids.iter().copied()).collect();
        self.db.set_marks(&observation_ids, true, Utc::now())?;
        Ok(unread.len())
    }

    /// Marks the entry read and returns its link; the user interface opens it.
    pub fn open_entry(&mut self, id: i64) -> Result<String> {
        let entry = self.resolve(&[id])?.remove(0);
        let Some(url) = entry.url.clone() else {
            bail!("entry {id} has no link");
        };
        self.db.set_marks(&entry.observation_ids, true, Utc::now())?;
        Ok(url)
    }

    /// A sync that can run on another thread.
    pub fn syncer(&self) -> Syncer {
        Syncer { config: self.config.clone(), db_path: self.db_path.clone() }
    }

    /// Syncs on this thread.
    pub fn sync(&self, on_event: impl FnMut(&SyncEvent)) -> Result<SyncSummary> {
        self.syncer().run(on_event)
    }

    /// Every configured source with the result of its last fetch.
    pub fn sources(&self) -> Result<Vec<SourceStatus>> {
        let fetches = self.db.fetches()?;
        Ok(self
            .config
            .sources
            .iter()
            .map(|s| {
                let fetch = fetches.get(&s.url);
                SourceStatus {
                    url: s.url.clone(),
                    title: entry::source_title(&s.url, Some(s), fetch),
                    tags: s.tags.clone(),
                    fetched_at: fetch.map(|f| f.fetched_at),
                    error: fetch.and_then(|f| f.error.clone()),
                }
            })
            .collect())
    }

    pub fn export_opml(&self) -> String {
        opml::export(&self.config)
    }
}

/// Validates a config file without opening the database.
pub fn check(config_path: &Path) -> Result<config::Loaded> {
    config::load(config_path)
}

/// Appends an OPML file's feeds to the config file, creating it if missing.
/// Nothing is written if the result would be invalid.
pub fn import_opml(config_path: &Path, opml_path: &Path) -> Result<ImportReport> {
    let opml_text = std::fs::read_to_string(opml_path).with_context(|| format!("cannot read {}", opml_path.display()))?;
    let config_text = match std::fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", config_path.display())),
    };
    let (text, report) = opml::import(&config_text, &opml_text)?;
    config::parse(&text).context("the imported config would be invalid; nothing was written")?;
    if let Some(dir) = config_path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(config_path, text).with_context(|| format!("cannot write {}", config_path.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewObservation;
    use chrono::Duration;

    const CONFIG: &str = r#"
        [[source]]
        url = "https://blog.example/feed"
        title = "Blog"
        tags = ["topic/os"]

        [[source]]
        url = "https://lobsters.example/feed"
        title = "Lobsters"
        tags = ["format/filter"]

        [[folder]]
        name = "OS"
        rule = "topic/os AND state/unread"

        [[folder]]
        name = "All"
        rule = "state"
    "#;

    fn setup() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("tagrss.toml");
        std::fs::write(&config_path, CONFIG).unwrap();
        let db_path = dir.path().join("tagrss.db");
        {
            let db = Database::open(&db_path).unwrap();
            let now = Utc::now();
            let add = |source: &str, key: &str, url: &str, days_ago: i64| {
                db.insert_observation(
                    &NewObservation {
                        source_url: source,
                        item_key: key,
                        url: Some(url),
                        title: key,
                        published_at: Some(now - Duration::days(days_ago)),
                        word_count: None,
                    },
                    now,
                )
                .unwrap();
            };
            add("https://blog.example/feed", "post", "https://blog.example/post", 2);
            add("https://lobsters.example/feed", "story", "https://blog.example/post/", 1);
            add("https://lobsters.example/feed", "other", "https://elsewhere.example/x", 1);
        }
        let (core, warnings) = Core::open(&config_path, &db_path).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        (dir, core)
    }

    #[test]
    fn first_sync_is_all_unread_and_duplicates_are_one_entry() {
        let (_dir, core) = setup();
        let folders = core.folders().unwrap();
        assert_eq!(folders[0], FolderSummary { name: "OS".into(), unread: 1, total: 1 });
        assert_eq!(folders[1], FolderSummary { name: "All".into(), unread: 2, total: 2 });
    }

    #[test]
    fn reading_via_either_observation_reads_the_entry() {
        let (_dir, mut core) = setup();
        let entry = core.entries("os").unwrap().remove(0);
        assert_eq!(entry.observation_ids.len(), 2);
        // Mark it through the Lobsters observation's id.
        core.mark(&[entry.observation_ids[1]], true).unwrap();
        assert_eq!(core.folders().unwrap()[0].unread, 0);
        assert!(core.entries("All").unwrap().iter().any(|e| e.id == entry.id && e.read));
        assert!(core.mark(&[9999], true).is_err());
    }

    #[test]
    fn open_entry_marks_read_and_returns_the_link() {
        let (_dir, mut core) = setup();
        let id = core.entries("OS").unwrap()[0].id;
        assert_eq!(core.open_entry(id).unwrap(), "https://blog.example/post");
        assert_eq!(core.folders().unwrap()[1].unread, 1);
    }

    #[test]
    fn mark_folder_read_marks_only_what_it_contains() {
        let (_dir, mut core) = setup();
        assert_eq!(core.mark_folder_read("OS").unwrap(), 1);
        let all = core.folders().unwrap()[1].clone();
        assert_eq!((all.unread, all.total), (1, 2));
        assert_eq!(core.mark_folder_read("All").unwrap(), 1);
        assert_eq!(core.folders().unwrap()[1].unread, 0);
    }

    #[test]
    fn an_invalid_reload_keeps_the_old_config() {
        let (dir, mut core) = setup();
        std::fs::write(dir.path().join("tagrss.toml"), "[[folder]]\nname = \"Bad\"\nrule = \"a AND\"\n").unwrap();
        assert!(core.reload_config().is_err());
        assert_eq!(core.config().folders.len(), 2);
        assert!(core.folder("nope").is_err());
    }
}
