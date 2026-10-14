use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use tagrss::{Core, SyncEvent};

mod tui;

#[derive(Parser)]
#[command(name = "tagrss", version, about = "A tag-based RSS reader. Run without a command to open the reader.")]
struct Cli {
    /// Config file [default: $XDG_CONFIG_HOME/tagrss/tagrss.toml]
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Database file [default: $XDG_DATA_HOME/tagrss/tagrss.db]
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch every source and store new entries
    Sync,
    /// Validate the config file and print how its rules parse
    Check,
    /// Append the feeds of an OPML file to the config file
    Import { opml: PathBuf },
    /// Print the config's sources as OPML
    Export,
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
    /// Mark entries read, or everything in a folder with --folder
    Read {
        #[arg(required_unless_present = "folder")]
        ids: Vec<i64>,
        #[arg(long, conflicts_with = "ids")]
        folder: Option<String>,
    },
    /// Mark entries unread
    Unread {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// List sources with the result of their last fetch
    Sources,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(tagrss::default_config_path);
    let db_path = cli.db.unwrap_or_else(tagrss::default_db_path);

    let Some(command) = cli.command else {
        let mut core = open(&config_path, &db_path)?;
        return tui::run(&mut core);
    };
    match command {
        Command::Check => check(&config_path),
        Command::Import { opml } => {
            let report = tagrss::import_opml(&config_path, &opml)?;
            println!("Added {} sources to {} ({} already present)", report.added, config_path.display(), report.already_present);
            for dropped in &report.dropped_tags {
                println!("  dropped tag {dropped}");
            }
            Ok(())
        }
        Command::Export => {
            print!("{}", open(&config_path, &db_path)?.export_opml());
            Ok(())
        }
        Command::Sync => sync(&open(&config_path, &db_path)?),
        Command::Folders => folders(&open(&config_path, &db_path)?),
        Command::List { folder, limit } => list(&open(&config_path, &db_path)?, &folder, limit),
        Command::Open { id } => {
            let url = open(&config_path, &db_path)?.open_entry(id)?;
            open_in_browser(&url)?;
            println!("{url}");
            Ok(())
        }
        Command::Read { ids, folder: Some(folder) } => {
            debug_assert!(ids.is_empty());
            let n = open(&config_path, &db_path)?.mark_folder_read(&folder)?;
            println!("Marked {n} entries read");
            Ok(())
        }
        Command::Read { ids, folder: None } => open(&config_path, &db_path)?.mark(&ids, true),
        Command::Unread { ids } => open(&config_path, &db_path)?.mark(&ids, false),
        Command::Sources => sources(&open(&config_path, &db_path)?),
    }
}

/// Opens the core, printing config warnings to stderr.
fn open(config_path: &Path, db_path: &Path) -> Result<Core> {
    let (core, warnings) = Core::open(config_path, db_path)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    Ok(core)
}

pub fn open_in_browser(url: &str) -> Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let status = std::process::Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .with_context(|| format!("cannot run {opener}"))?;
    if !status.success() {
        bail!("{opener} failed for {url}");
    }
    Ok(())
}

fn check(path: &Path) -> Result<()> {
    let loaded = tagrss::check(path)?;
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

fn sync(core: &Core) -> Result<()> {
    let titles: std::collections::HashMap<String, String> =
        core.sources()?.into_iter().map(|s| (s.url, s.title)).collect();
    let summary = core.sync(|event| {
        if let SyncEvent::Failed { url, error } = event {
            eprintln!("failed  {}: {error}", titles.get(url).unwrap_or(url));
        }
    })?;
    println!("{} new entries from {} sources, {} failed", summary.new_observations, summary.sources, summary.failed);
    Ok(())
}

fn folders(core: &Core) -> Result<()> {
    let folders = core.folders()?;
    if folders.is_empty() {
        println!("No folders defined. Add [[folder]] entries to the config file.");
        return Ok(());
    }
    let width = folders.iter().map(|f| f.name.width()).max().unwrap_or(0).max(6);
    println!("{}  {:>6}  {:>6}", pad("Folder", width), "Unread", "Total");
    for f in folders {
        println!("{}  {:>6}  {:>6}", pad(&f.name, width), f.unread, f.total);
    }
    Ok(())
}

fn list(core: &Core, folder: &str, limit: usize) -> Result<()> {
    let entries = core.entries(folder)?;
    if entries.is_empty() {
        println!("No entries in \"{folder}\".");
        return Ok(());
    }
    let shown = &entries[..entries.len().min(limit)];
    let id_width = shown.iter().map(|e| e.id.to_string().len()).max().unwrap_or(1);
    for e in shown {
        let marker = if e.read { ' ' } else { '*' };
        println!(
            "{marker} {:>id_width$}  {}  {}  {}",
            e.id,
            e.date.format("%Y-%m-%d"),
            pad(&e.sources.join(", "), 22),
            fit(&e.title, 80)
        );
    }
    Ok(())
}

fn sources(core: &Core) -> Result<()> {
    for s in core.sources()? {
        let state = match (&s.fetched_at, &s.error) {
            (None, _) => "never fetched".to_string(),
            (Some(_), Some(e)) => format!("failed: {e}"),
            (Some(t), None) => format!("ok {}", t.format("%Y-%m-%d %H:%M")),
        };
        println!("{}  {state}", pad(&s.title, 40));
    }
    Ok(())
}

/// Truncates to at most `width` display columns, ending with "…" when cut.
pub fn fit(s: &str, width: usize) -> String {
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
pub fn pad(s: &str, width: usize) -> String {
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
