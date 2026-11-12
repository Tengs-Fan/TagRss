use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use tagrss::config::{self, Config, Folder};
use tagrss::db::Database;
use tagrss::feed::Outcome;
use tagrss::{feed, opml, tags, view};

/// `println!` that ends the process quietly when stdout is a closed pipe, as in `tagrss list Today | head`.
/// The standard macro panics there.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write;
        if writeln!(std::io::stdout(), $($arg)*).is_err() {
            std::process::exit(0);
        }
    }};
}

/// `sync` lists failed sources itself up to this many; beyond it, it points at `sources --failed`.
const FAILURES_SHOWN_BY_SYNC: usize = 10;

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
    /// Mark entries read: the given ids, or everything in a folder
    Read {
        #[arg(required_unless_present = "folder")]
        ids: Vec<i64>,
        /// Mark every entry in this folder instead
        #[arg(short, long, conflicts_with = "ids")]
        folder: Option<String>,
    },
    /// Mark entries unread: the given ids, or everything in a folder
    Unread {
        #[arg(required_unless_present = "folder")]
        ids: Vec<i64>,
        /// Mark every entry in this folder instead
        #[arg(short, long, conflicts_with = "ids")]
        folder: Option<String>,
    },
    /// List sources with their source/ tag, entry count, and last sync
    Sources {
        /// Only sources whose last sync failed, with the error
        #[arg(long)]
        failed: bool,
    },
    /// Validate the config file and print how its rules parse
    Check,
    /// Append the feeds of an OPML file to the config file
    Import { opml: PathBuf },
    /// Print the config's sources as OPML
    Export,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", describe(&e));
            ExitCode::FAILURE
        }
    }
}

/// An error with its causes on one line; a cause that is itself a list (config errors) starts on a new line.
fn describe(error: &anyhow::Error) -> String {
    let mut out = String::new();
    for (i, cause) in error.chain().enumerate() {
        let cause = cause.to_string();
        if i == 0 {
            out.push_str(&cause);
        } else if cause.contains('\n') {
            out.push_str(":\n  ");
            out.push_str(&cause.replace('\n', "\n  "));
        } else {
            out.push_str(": ");
            out.push_str(&cause);
        }
    }
    out
}

async fn run(cli: Cli) -> Result<()> {
    let config_path = cli.config.unwrap_or_else(config::default_config_path);
    let db_path = cli.db.unwrap_or_else(config::default_db_path);

    match cli.command {
        Command::Check => check(&config_path),
        Command::Import { opml } => import(&config_path, &opml),
        Command::Export => {
            out!("{}", opml::export(&load_config(&config_path)?).trim_end());
            Ok(())
        }
        Command::Sync => sync(&load_config(&config_path)?, &open_db(&db_path)?).await,
        Command::Folders => folders(&load_config(&config_path)?, &open_db(&db_path)?),
        Command::List { folder, limit } => list(&load_config(&config_path)?, &open_db(&db_path)?, &folder, limit),
        Command::Open { id } => open(&open_db(&db_path)?, id),
        Command::Read { ids, folder } => mark(&config_path, &open_db(&db_path)?, &ids, folder.as_deref(), true),
        Command::Unread { ids, folder } => mark(&config_path, &open_db(&db_path)?, &ids, folder.as_deref(), false),
        Command::Sources { failed } => sources(&load_config(&config_path)?, &open_db(&db_path)?, failed),
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
    out!("{}: {} sources, {} folders", path.display(), c.sources.len(), c.folders.len());
    let width = c.folders.iter().map(|f| f.name.width()).max().unwrap_or(0);
    for f in &c.folders {
        out!("  {}  {}", pad(&f.name, width), f.rule);
    }
    for w in &loaded.warnings {
        out!("warning: {w}");
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
    out!(
        "Added {} to {} ({} already present)",
        count(report.added, "source", "sources"),
        config_path.display(),
        report.already_present
    );
    for dropped in &report.dropped_tags {
        out!("  dropped tag {dropped}");
    }
    Ok(())
}

async fn sync(config: &Config, db: &Database) -> Result<()> {
    let now = Utc::now();
    // Hundreds of feeds take a while; a counter on a terminal shows it is alive.
    let show_progress = std::io::stderr().is_terminal();
    let results = feed::sync(config, db, now, |done, total| {
        if show_progress {
            eprint!("\rFetching {done}/{total}");
        }
    })
    .await?;
    if show_progress {
        eprint!("\r\x1b[K");
    }

    let (mut added, mut unchanged) = (0, 0);
    let mut failures = Vec::new();
    for r in &results {
        match &r.outcome {
            Outcome::New(n) => added += n,
            Outcome::Unchanged => unchanged += 1,
            Outcome::Failed(error) => failures.push((&r.url, error)),
        }
    }
    if failures.len() <= FAILURES_SHOWN_BY_SYNC {
        let fetches = db.fetches()?;
        for (url, error) in &failures {
            let source = config.sources.iter().find(|s| s.url == **url);
            let title = view::source_title(url, source, fetches.get(*url));
            eprintln!("failed  {title}: {error}");
        }
    }
    out!(
        "{} from {} ({unchanged} unchanged, {} failed)",
        count(added, "new entry", "new entries"),
        count(results.len(), "source", "sources"),
        failures.len()
    );
    if failures.len() > FAILURES_SHOWN_BY_SYNC {
        out!("Run `tagrss sources --failed` to see the failures.");
    }
    Ok(())
}

fn sources(config: &Config, db: &Database, only_failed: bool) -> Result<()> {
    let all = view::sources(config, db)?;
    let is_failed = |s: &view::SourceStatus| s.fetch.as_ref().is_some_and(|f| f.error.is_some());
    let failed = all.iter().filter(|s| is_failed(s)).count();
    let never = all.iter().filter(|s| s.fetch.is_none()).count();

    let rows: Vec<(String, &view::SourceStatus)> = all
        .iter()
        .filter(|s| !only_failed || is_failed(s))
        .map(|s| (format!("source/{}", tags::slug(&s.title)), s))
        .collect();
    if !rows.is_empty() {
        // The tag comes last and is never cut: it is what gets typed into a rule.
        out!("{:>7}  {:<10}  {:<17}  Source", "Entries", "Newest", "Last sync");
        for (tag, s) in &rows {
            let newest = s.newest.map_or("-".to_string(), |d| d.format("%Y-%m-%d").to_string());
            let (status, error) = match &s.fetch {
                None => ("never".to_string(), None),
                Some(f) => {
                    let date = f.fetched_at.format("%Y-%m-%d");
                    match &f.error {
                        None => (format!("{date} ok"), None),
                        Some(e) => (format!("{date} failed"), Some(e)),
                    }
                }
            };
            out!("{:>7}  {newest:<10}  {status:<17}  {tag}", s.entries);
            if let Some(error) = error {
                out!("         {}", s.source.url);
                out!("         {error}");
            }
        }
    }
    let ok = all.len() - failed - never;
    out!("{}: {ok} ok, {failed} failed, {never} never synced", count(all.len(), "source", "sources"));
    Ok(())
}

fn folders(config: &Config, db: &Database) -> Result<()> {
    if config.folders.is_empty() {
        out!("No folders defined. Add [[folder]] entries to the config file.");
        return Ok(());
    }
    let items = view::items(config, db, Utc::now())?;
    let width = config.folders.iter().map(|f| f.name.width()).max().unwrap_or(0);
    out!("{}  {:>6}  {:>6}", pad("Folder", width), "Unread", "Total");
    for f in &config.folders {
        let (mut unread, mut total) = (0, 0);
        for item in view::in_folder(&items, f) {
            total += 1;
            unread += usize::from(!item.read);
        }
        out!("{}  {unread:>6}  {total:>6}", pad(&f.name, width));
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
        out!("No entries in \"{}\".", folder.name);
        return Ok(());
    }
    let id_width = shown.iter().map(|i| i.entry.id.to_string().len()).max().unwrap_or(1);
    for item in shown {
        let e = &item.entry;
        let date = e.published_at.unwrap_or(e.first_seen_at).format("%Y-%m-%d");
        let marker = if item.read { ' ' } else { '*' };
        out!(
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
    out!("{url}");
    Ok(())
}

fn mark(config_path: &Path, db: &Database, ids: &[i64], folder: Option<&str>, read: bool) -> Result<()> {
    let now = Utc::now();
    let Some(name) = folder else {
        let mut missing = Vec::new();
        db.transaction(|| {
            for &id in ids {
                if !db.set_mark(id, read, now)? {
                    missing.push(id.to_string());
                }
            }
            Ok(())
        })?;
        if !missing.is_empty() {
            bail!("no entry {}", missing.join(", "));
        }
        return Ok(());
    };

    // Marking by id needs no config; marking a folder does, for its rule.
    let config = load_config(config_path)?;
    let folder = find_folder(&config, name)?;
    let items = view::items(&config, db, now)?;
    let to_mark: Vec<i64> = view::in_folder(&items, folder).filter(|i| i.read != read).map(|i| i.entry.id).collect();
    db.transaction(|| {
        for &id in &to_mark {
            db.set_mark(id, read, now)?;
        }
        Ok(())
    })?;
    let state = if read { "read" } else { "unread" };
    out!("Marked {} {state} in \"{}\"", count(to_mark.len(), "entry", "entries"), folder.name);
    Ok(())
}

/// "1 entry", "2 entries".
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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

    #[test]
    fn errors_are_one_line_unless_a_cause_is_a_list() {
        let simple = anyhow::anyhow!("No such file").context("cannot read x");
        assert_eq!(describe(&simple), "cannot read x: No such file");
        let list = anyhow::anyhow!("source #1: bad\nfolder #2: worse").context("invalid config file x");
        assert_eq!(describe(&list), "invalid config file x:\n  source #1: bad\n  folder #2: worse");
    }
}
