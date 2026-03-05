mod config;
mod db;
mod feed;
mod folder;
mod models;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::path::Path;

use db::Database;
use folder::{Expr, Folder};
use models::Rule;

const DB_PATH: &str = "tagrss.db";
const FEEDS_PATH: &str = "configs/feeds.opml";
const RULES_PATH: &str = "configs/rules.yaml";
const FOLDERS_PATH: &str = "configs/folders.yaml";

#[derive(Parser)]
#[command(name = "tagrss")]
#[command(about = "Tag-based RSS reader")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new source with tags
    Add {
        url: String,
        #[arg(short, long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// List all sources
    Sources,
    /// Set tags for a source
    SetTags {
        source_id: i64,
        #[arg(value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// Sync all sources (fetch new articles)
    Sync,
    /// List articles, optionally filtered by folder
    List {
        #[arg(short, long)]
        folder: Option<String>,
        #[arg(short, long)]
        unread: bool,
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Show all tags in use
    Tags,
    /// Rule management
    Rule {
        #[command(subcommand)]
        cmd: RuleCmd,
    },
    /// Folder management
    Folder {
        #[command(subcommand)]
        cmd: FolderCmd,
    },
    /// Mark article as read
    Read { article_id: i64 },
    /// Import sources from OPML and rules from YAML
    Import {
        /// OPML file path (default: configs/feeds.opml)
        #[arg(long)]
        feeds: Option<String>,
        /// Rules YAML file path (default: configs/rules.yaml)
        #[arg(long)]
        rules: Option<String>,
    },
    /// Reload folders from YAML config
    Reload,
}

#[derive(Subcommand)]
enum RuleCmd {
    /// List all rules
    List,
    /// Add a contains rule
    AddContains {
        pattern: String,
        #[arg(short, long)]
        tag: String,
        #[arg(long)]
        case_sensitive: bool,
    },
    /// Add a word count rule
    AddWordCount {
        #[arg(long)]
        min: Option<u32>,
        #[arg(long)]
        max: Option<u32>,
        #[arg(short, long)]
        tag: String,
    },
    /// Add an age rule
    AddAge {
        #[arg(long)]
        max_days: Option<u32>,
        #[arg(long)]
        min_days: Option<u32>,
        #[arg(short, long)]
        tag: String,
    },
    /// Delete a rule
    Delete { rule_id: i64 },
    /// Apply rules to all existing articles
    Apply,
    /// Import rules from YAML file
    Import {
        #[arg(default_value = "configs/rules.yaml")]
        path: String,
    },
}

#[derive(Subcommand)]
enum FolderCmd {
    /// List all folders
    List,
    /// Add a folder
    Add {
        name: String,
        /// Filter expression, e.g. "important AND NOT long"
        filter: String,
    },
    /// Test a filter expression
    Test { filter: String },
}

fn load_folders() -> Vec<Folder> {
    if Path::new(FOLDERS_PATH).exists() {
        config::load_folders(FOLDERS_PATH).unwrap_or_default()
    } else {
        Vec::new()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let db = Database::open(DB_PATH)?;

    match cli.command {
        Commands::Add { url, tags } => {
            println!("Fetching {}...", url);
            let (title, _) = feed::fetch_feed(&url).await?;
            let tags: HashSet<String> = tags.into_iter().collect();
            let id = db.add_source(&url, &title, &tags)?;
            println!("Added source #{}: {} (tags: {:?})", id, title, tags);
        }

        Commands::Sources => {
            let sources = db.get_sources()?;
            if sources.is_empty() {
                println!("No sources. Use 'tagrss import' to load from {}", FEEDS_PATH);
                return Ok(());
            }
            println!("{:<4} {:<40} {}", "ID", "Title", "Tags");
            println!("{}", "-".repeat(80));
            for s in sources {
                let tags: Vec<_> = s.tags.iter().collect();
                println!("{:<4} {:<40} {:?}", s.id, truncate(&s.title, 38), tags);
            }
        }

        Commands::SetTags { source_id, tags } => {
            let tags: HashSet<String> = tags.into_iter().collect();
            db.update_source_tags(source_id, &tags)?;
            println!("Updated source #{} tags to {:?}", source_id, tags);
        }

        Commands::Sync => {
            let sources = db.get_sources()?;
            let rules = db.get_rules()?;
            if sources.is_empty() {
                println!("No sources to sync. Use 'tagrss import' first.");
                return Ok(());
            }
            for source in &sources {
                print!("Syncing {}... ", source.title);
                match feed::sync_source(&db, source, &rules).await {
                    Ok(n) => println!("{} new articles", n),
                    Err(e) => println!("error: {}", e),
                }
            }
        }

        Commands::List {
            folder,
            unread,
            limit,
        } => {
            let articles = db.get_articles()?;
            let folders = load_folders();

            let filter: Option<Expr> = if let Some(name) = &folder {
                folders
                    .iter()
                    .find(|f| f.name == *name)
                    .map(|f| f.filter.clone())
            } else {
                None
            };

            let filtered: Vec<_> = articles
                .into_iter()
                .filter(|a| !unread || !a.read)
                .filter(|a| filter.as_ref().map_or(true, |f| f.matches(a)))
                .take(limit)
                .collect();

            if filtered.is_empty() {
                println!("No articles found.");
                return Ok(());
            }

            println!(
                "{:<4} {:<50} {:<20} {}",
                "ID", "Title", "Tags", "Read"
            );
            println!("{}", "-".repeat(90));
            for a in filtered {
                let tags: Vec<_> = a.tags.iter().take(3).collect();
                let read_mark = if a.read { "[x]" } else { "[ ]" };
                println!(
                    "{:<4} {:<50} {:<20} {}",
                    a.id,
                    truncate(&a.title, 48),
                    format!("{:?}", tags),
                    read_mark
                );
            }
        }

        Commands::Tags => {
            let articles = db.get_articles()?;
            let mut all_tags: HashSet<String> = HashSet::new();
            for a in &articles {
                all_tags.extend(a.tags.iter().cloned());
            }
            let mut tags: Vec<_> = all_tags.into_iter().collect();
            tags.sort();
            println!("Tags in use:");
            for t in tags {
                println!("  {}", t);
            }
        }

        Commands::Rule { cmd } => match cmd {
            RuleCmd::List => {
                let rules = db.get_rules()?;
                if rules.is_empty() {
                    println!("No rules defined. Use 'tagrss rule import' to load from {}", RULES_PATH);
                    return Ok(());
                }
                for (id, rule) in rules {
                    println!("#{}: {:?}", id, rule);
                }
            }
            RuleCmd::AddContains {
                pattern,
                tag,
                case_sensitive,
            } => {
                let rule = Rule::Contains {
                    pattern,
                    case_sensitive,
                    tag: tag.clone(),
                };
                let id = db.add_rule(&rule)?;
                println!("Added rule #{} -> tag '{}'", id, tag);
            }
            RuleCmd::AddWordCount { min, max, tag } => {
                let rule = Rule::WordCount {
                    min,
                    max,
                    tag: tag.clone(),
                };
                let id = db.add_rule(&rule)?;
                println!("Added rule #{} -> tag '{}'", id, tag);
            }
            RuleCmd::AddAge {
                max_days,
                min_days,
                tag,
            } => {
                let rule = Rule::Age {
                    max_days,
                    min_days,
                    tag: tag.clone(),
                };
                let id = db.add_rule(&rule)?;
                println!("Added rule #{} -> tag '{}'", id, tag);
            }
            RuleCmd::Delete { rule_id } => {
                db.delete_rule(rule_id)?;
                println!("Deleted rule #{}", rule_id);
            }
            RuleCmd::Apply => {
                let rules = db.get_rules()?;
                let sources = db.get_sources()?;
                let articles = db.get_articles()?;
                let mut updated = 0;

                for article in articles {
                    let source = sources.iter().find(|s| s.id == article.source_id);
                    let mut tags = source.map(|s| s.tags.clone()).unwrap_or_default();

                    for (_, rule) in &rules {
                        if let Some(tag) = rule.apply(&article) {
                            tags.insert(tag);
                        }
                    }

                    if tags != article.tags {
                        db.update_article_tags(article.id, &tags)?;
                        updated += 1;
                    }
                }
                println!("Updated {} articles", updated);
            }
            RuleCmd::Import { path } => {
                let rules = config::load_rules(&path)?;
                println!("Loading rules from {}...", path);
                for rule in &rules {
                    db.add_rule(rule)?;
                    println!("  Added: {:?}", rule);
                }
                println!("Imported {} rules", rules.len());
            }
        },

        Commands::Folder { cmd } => match cmd {
            FolderCmd::List => {
                let folders = load_folders();
                if folders.is_empty() {
                    println!("No folders defined. Edit {} to add folders.", FOLDERS_PATH);
                    return Ok(());
                }
                for f in folders {
                    println!("{}: {:?}", f.name, f.filter);
                }
            }
            FolderCmd::Add { name, filter } => {
                let _ = Expr::parse(&filter).map_err(|e| anyhow::anyhow!(e))?;
                println!("To add folder '{}', edit {} and add:", name, FOLDERS_PATH);
                println!("  - name: {}", name);
                println!("    filter: {}", filter);
            }
            FolderCmd::Test { filter } => {
                let expr = Expr::parse(&filter).map_err(|e| anyhow::anyhow!(e))?;
                println!("Parsed: {:?}", expr);
                let articles = db.get_articles()?;
                let matching: Vec<_> = articles.iter().filter(|a| expr.matches(a)).collect();
                println!("Matches {} articles", matching.len());
            }
        },

        Commands::Read { article_id } => {
            db.mark_read(article_id, true)?;
            println!("Marked article #{} as read", article_id);
        }

        Commands::Import { feeds, rules } => {
            let feeds_path = feeds.as_deref().unwrap_or(FEEDS_PATH);
            let rules_path = rules.as_deref().unwrap_or(RULES_PATH);

            // Import feeds from OPML
            if Path::new(feeds_path).exists() {
                println!("Loading feeds from {}...", feeds_path);
                let opml_feeds = config::load_opml(feeds_path)?;

                for opml_feed in &opml_feeds {
                    print!("  {} ... ", opml_feed.title);
                    match db.add_source(&opml_feed.xml_url, &opml_feed.title, &opml_feed.tags) {
                        Ok(id) => println!("OK (#{})", id),
                        Err(e) => println!("skip ({})", e),
                    }
                }
                println!("Imported {} feeds", opml_feeds.len());
            } else {
                println!("No {} found, skipping feeds import", feeds_path);
            }

            // Import rules from YAML
            if Path::new(rules_path).exists() {
                println!("\nLoading rules from {}...", rules_path);
                let rules = config::load_rules(rules_path)?;
                for rule in &rules {
                    db.add_rule(rule)?;
                    println!("  Added: {:?}", rule);
                }
                println!("Imported {} rules", rules.len());
            } else {
                println!("No {} found, skipping rules import", rules_path);
            }

            // Check folders
            if Path::new(FOLDERS_PATH).exists() {
                let folders = config::load_folders(FOLDERS_PATH)?;
                println!("\nLoaded {} folders from {}", folders.len(), FOLDERS_PATH);
            } else {
                println!("\nNo {} found", FOLDERS_PATH);
            }

            println!("\nDone! Now run 'tagrss sync' to fetch articles.");
        }

        Commands::Reload => {
            if Path::new(FOLDERS_PATH).exists() {
                let folders = config::load_folders(FOLDERS_PATH)?;
                println!("Reloaded {} folders from {}", folders.len(), FOLDERS_PATH);
                for f in &folders {
                    println!("  {}: {:?}", f.name, f.filter);
                }
            } else {
                println!("No {} found", FOLDERS_PATH);
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
