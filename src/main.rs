use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use tagrss::config::{self, Config, Folder};
use tagrss::db::Database;
use tagrss::{feed, opml, view};

#[derive(Parser)]
#[command(name = "tagrss", version, about = "A tag-based RSS reader")]
struct Cli {
    /// Config file [default: $XDG_CONFIG_HOME/tagrss/tagrss.toml]
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Database file [default: $XDG_DATA_HOME/tagrss/tagrss.db]
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch every source and store new entries
    Sync,
    /// List folders with their unread counts
    Folders,
    /// List a folder's entries, newest first
    List {
        folder: String,
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: usize,
    },
    /// Open an entry's link in the browser and mark it read
    Open { id: i64 },
    /// Mark entries read
    Read {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// Mark entries unread
    Unread {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// Validate the config file and print how its rules parse
    Check,
    /// Append the feeds of an OPML file to the config file
    Import { opml: PathBuf },
    /// Print the config's sources as OPML
    Export,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(config::default_config_path);
    let db_path = cli.db.unwrap_or_else(config::default_db_path);

    match cli.command {
        Command::Check => check(&config_path),
        Command::Import { opml } => import(&config_path, &opml),
        Command::Export => {
            print!("{}", opml::export(&load_config(&config_path)?));
            Ok(())
        }
        Command::Sync => sync(&load_config(&config_path)?, &open_db(&db_path)?).await,
        Command::Folders => folders(&load_config(&config_path)?, &open_db(&db_path)?),
        Command::List { folder, limit } => list(&load_config(&config_path)?, &open_db(&db_path)?, &folder, limit),
        Command::Open { id } => open(&open_db(&db_path)?, id),
        Command::Read { ids } => mark(&open_db(&db_path)?, &ids, true),
        Command::Unread { ids } => mark(&open_db(&db_path)?, &ids, false),
    }
}

/// Loads the config, printing warnings to stderr.
fn load_config(path: &Path) -> Result<Config> {
    let loaded = config::load(path)?;
    for w in &loaded.warnings {
        eprintln!("warning: {w}");
    }
    Ok(loaded.config)
}

fn open_db(path: &Path) -> Result<Database> {
    Database::open(path, Utc::now())
}

fn check(path: &Path) -> Result<()> {
    let loaded = config::load(path)?;
    let c = &loaded.config;
    println!("{}: {} sources, {} folders", path.display(), c.sources.len(), c.folders.len());
    let width = c.folders.iter().map(|f| f.name.width()).max().unwrap_or(0);
    for f in &c.folders {
        println!("  {}  {}", pad(&f.name, width), f.rule);
    }
    for w in &loaded.warnings {
        println!("warning: {w}");
    }
    Ok(())
}

fn import(config_path: &Path, opml_path: &Path) -> Result<()> {
    let opml_text = std::fs::read_to_string(opml_path)
        .with_context(|| format!("cannot read {}", opml_path.display()))?;
    let config_text = match std::fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", config_path.display())),
    };
    let (text, report) = opml::import(&config_text, &opml_text)?;
    config::parse(&text).context("the imported config would be invalid; nothing was written")?;
    if let Some(dir) = config_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(config_path, text).with_context(|| format!("cannot write {}", config_path.display()))?;
    println!(
        "Added {} sources to {} ({} already present)",
        report.added,
        config_path.display(),
        report.already_present
    );
    for dropped in &report.dropped_tags {
        println!("  dropped tag {dropped}");
    }
    Ok(())
}

async fn sync(config: &Config, db: &Database) -> Result<()> {
    let now = Utc::now();
    let results = feed::sync(config, db, now).await?;
    let fetches = db.fetches()?;
    let (mut added, mut failed) = (0, 0);
    for r in &results {
        match &r.outcome {
            Ok(n) => added += n,
            Err(e) => {
                failed += 1;
                let source = config.sources.iter().find(|s| s.url == r.url);
                let title = view::source_title(&r.url, source, fetches.get(&r.url));
                eprintln!("failed  {title}: {e:#}");
            }
        }
    }
    println!("{added} new entries from {} sources, {failed} failed", results.len());
    Ok(())
}

fn folders(config: &Config, db: &Database) -> Result<()> {
    if config.folders.is_empty() {
        println!("No folders defined. Add [[folder]] entries to the config file.");
        return Ok(());
    }
    let items = view::items(config, db, Utc::now())?;
    let width = config.folders.iter().map(|f| f.name.width()).max().unwrap_or(0);
    println!("{}  {:>6}  {:>6}", pad("Folder", width), "Unread", "Total");
    for f in &config.folders {
        let (mut unread, mut total) = (0, 0);
        for item in view::in_folder(&items, f) {
            total += 1;
            unread += usize::from(!item.read);
        }
        println!("{}  {unread:>6}  {total:>6}", pad(&f.name, width));
    }
    Ok(())
}

fn find_folder<'a>(config: &'a Config, name: &str) -> Result<&'a Folder> {
    if let Some(f) = config.folders.iter().find(|f| f.name == name) {
        return Ok(f);
    }
    let lower = name.to_lowercase();
    let close: Vec<&Folder> = config.folders.iter().filter(|f| f.name.to_lowercase() == lower).collect();
    if let [only] = close.as_slice() {
        return Ok(only);
    }
    let names: Vec<&str> = config.folders.iter().map(|f| f.name.as_str()).collect();
    bail!("no folder named \"{name}\"; folders are: {}", names.join(", "))
}

fn list(config: &Config, db: &Database, name: &str, limit: usize) -> Result<()> {
    let folder = find_folder(config, name)?;
    let items = view::items(config, db, Utc::now())?;
    let shown: Vec<_> = view::in_folder(&items, folder).take(limit).collect();
    if shown.is_empty() {
        println!("No entries in \"{}\".", folder.name);
        return Ok(());
    }
    let id_width = shown.iter().map(|i| i.entry.id.to_string().len()).max().unwrap_or(1);
    for item in shown {
        let e = &item.entry;
        let date = e.published_at.unwrap_or(e.first_seen_at).format("%Y-%m-%d");
        let marker = if item.read { ' ' } else { '*' };
        println!(
            "{marker} {:>id_width$}  {date}  {}  {}",
            e.id,
            pad(&item.source_title, 22),
            fit(&e.title, 80)
        );
    }
    Ok(())
}

fn open(db: &Database, id: i64) -> Result<()> {
    let Some(entry) = db.entry(id)? else {
        bail!("no entry {id}");
    };
    let Some(url) = entry.url else {
        bail!("entry {id} has no link");
    };
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let status = std::process::Command::new(opener).arg(&url).status()
        .with_context(|| format!("cannot run {opener}"))?;
    if !status.success() {
        bail!("{opener} failed for {url}");
    }
    db.set_mark(id, true, Utc::now())?;
    println!("{url}");
    Ok(())
}

fn mark(db: &Database, ids: &[i64], read: bool) -> Result<()> {
    let now = Utc::now();
    let missing: Vec<String> = ids
        .iter()
        .filter_map(|&id| match db.set_mark(id, read, now) {
            Ok(true) => None,
            Ok(false) => Some(Ok(id.to_string())),
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<_>>()?;
    if !missing.is_empty() {
        bail!("no entry {}", missing.join(", "));
    }
    Ok(())
}

/// Truncates to at most `width` display columns, ending with "…" when cut.
fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Truncates or pads to exactly `width` display columns.
fn pad(s: &str, width: usize) -> String {
    let fitted = fit(s, width);
    let fill = width.saturating_sub(fitted.width());
    format!("{fitted}{}", " ".repeat(fill))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_and_pad_count_display_columns() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("abcdefghij", 5), "abcd…");
        // Each Han character is two columns wide; slicing never splits a character.
        assert_eq!(fit("跟李沐学AI的动态", 7), "跟李沐…");
        assert_eq!(pad("跟李", 6), "跟李  ");
        assert_eq!(pad("abc", 3), "abc");
    }
}
