use clap::{Parser, Subcommand};
use anyhow::Result;
use chrono;
use log::{info, debug, warn, error, LevelFilter};

mod db;
mod feed;
mod models;
mod tag;
mod logger;
mod folder;

use tag::{TagManager, TagRuleEnum, Contains, TimeRange };
use logger::{LogConfig, parse_log_level};
use folder::FolderManager;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Set the console log level (off, error, warn, info, debug, trace)
    #[arg(long, default_value = "info")]
    log_level: String,
    
    /// Enable logging to a file
    #[arg(long)]
    log_file: Option<String>,
    
    /// Set the file log level (off, error, warn, info, debug, trace)
    #[arg(long, default_value = "debug")]
    file_log_level: String,
    
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Add a new feed
    #[command(name = "add")]
    AddFeed {
        /// URL of the feed to add
        url: String,
    },
    
    /// List all feeds
    #[command(name = "list")]
    ListFeeds,
    
    /// Update all feeds
    #[command(name = "update")]
    UpdateFeeds,
    
    /// Manage tag rules
    #[command(name = "rules")]
    Rules {
        #[command(subcommand)]
        subcommand: Option<RuleCommands>,
    },
    
    /// Manage folders
    #[command(name = "folders")]
    Folders {
        #[command(subcommand)]
        subcommand: Option<FolderCommands>,
    },
}

#[derive(Subcommand, Debug)]
enum RuleCommands {
    /// List all rules
    #[command(name = "list")]
    List,
    
    /// Add a new contains rule
    #[command(name = "add-contains")]
    AddContains {
        /// Name of the tag
        name: String,
        
        /// String to search for
        target: String,
        
        /// Whether the search is case sensitive
        #[arg(long, default_value = "false")]
        case_sensitive: bool,
    },
    
    /// Add a new time range rule
    #[command(name = "add-timerange")]
    AddTimeRange {
        /// Name of the tag
        name: String,
        
        /// Start date (ISO format)
        #[arg(long)]
        start: Option<String>,
        
        /// End date (ISO format)
        #[arg(long)]
        end: Option<String>,
    },
    
    // /// Add a new from-feed rule
    // #[command(name = "add-feed")]
    // AddFromFeed {
    //     /// Name of the tag
    //     name: String,
        
    //     /// ID of the feed
    //     feed_id: i64,
    // },
    
    /// Apply rules to existing items
    #[command(name = "apply")]
    Apply,
}

#[derive(Subcommand, Debug)]
enum FolderCommands {
    /// List all folders
    #[command(name = "list")]
    List,
    
    /// Reload folders from the YAML configuration file
    #[command(name = "reload")]
    Reload,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize logger with command line options
    let log_config = LogConfig {
        console_level: parse_log_level(&args.log_level),
        file_level: parse_log_level(&args.file_log_level),
        log_file: args.log_file,
    };
    
    if let Err(e) = logger::init(log_config) {
        eprintln!("Warning: Failed to initialize logger: {}", e);
    }
    
    // Log startup info
    info!("TagRss starting");
    debug!("Debug logging enabled");
    
    // Initialize database
    let db = db::Database::new("sqlite:tagrss.db").await?;
    
    // Initialize tag manager
    let tag_manager = TagManager::new("tag_rules.json");
    
    // Initialize folder manager with YAML config
    let mut folder_manager = FolderManager::new("folders.yml");
    
    // Initialize feed manager with tag manager
    let mut feed_manager = feed::FeedManager::new(db, tag_manager);
    
    match args.command {
        Some(Commands::AddFeed { url }) => {
            info!("Adding feed: {}", url);
            feed_manager.add_feed(&url).await?;
        }
        
        Some(Commands::ListFeeds) => {
            info!("Listing feeds:");
            let feeds = feed_manager.db.get_feeds().await?;
            for (id, url, title) in feeds {
                let title = title.unwrap_or_else(|| "no title".to_string());
                info!("ID: {}, URL: {}, Title: {}", id, url, title);
            }
        }
        
        Some(Commands::UpdateFeeds) => {
            info!("Updating feeds:");
            feed_manager.update_feeds().await?;
            info!("Feeds updated successfully.");
        }
        
        Some(Commands::Rules { subcommand }) => {
            match subcommand {
                Some(RuleCommands::List) => {
                    info!("Listing rules:");
                    for (i, rule) in feed_manager.tag_manager.rules().iter().enumerate() {
                        info!("Rule {}: {:?}", i + 1, rule);
                    }
                }
                
                Some(RuleCommands::AddContains { name, target, case_sensitive }) => {
                    info!("Adding contains rule: {} -> {}", name, target);
                    
                    // Create simplified regex pattern
                    let regex_pattern = if case_sensitive {
                        // Use plain string as regex pattern
                        target.clone()
                    } else {
                        // Add case-insensitive flag for regex
                        format!("(?i){}", target)
                    };
                    
                    let tag = tag::Tag::new(name);
                    
                    let rule = Contains {
                        tag,
                        target_regex: regex_pattern,
                    };
                    
                    feed_manager.tag_manager.add_rule(TagRuleEnum::Contains(rule));
                    feed_manager.tag_manager.save_to_file()?;
                }
                
                Some(RuleCommands::AddTimeRange { name, start, end }) => {
                    info!("Adding time-range rule: {} -> {} to {}", 
                        name, 
                        start.as_ref().map_or("anytime".to_string(), |d| d.to_string()), 
                        end.as_ref().map_or("anytime".to_string(), |d| d.to_string())
                    );
                    
                    // Create a tag with just a name
                    let tag = tag::Tag::new(name);
                    
                    // Parse dates if provided
                    let start_date = match &start {
                        Some(s) => Some(chrono::DateTime::parse_from_rfc3339(s)?.with_timezone(&chrono::Utc)),
                        None => None
                    };
                    
                    let end_date = match &end {
                        Some(e) => Some(chrono::DateTime::parse_from_rfc3339(e)?.with_timezone(&chrono::Utc)),
                        None => None
                    };
                    
                    let rule = TimeRange {
                        tag,
                        start: start_date,
                        end: end_date,
                    };
                    
                    feed_manager.tag_manager.add_rule(TagRuleEnum::TimeRange(rule));
                    feed_manager.tag_manager.save_to_file()?;
                }
                
                // Some(RuleCommands::AddFromFeed { name, feed_id }) => {
                //     info!("Adding from-feed rule: {} -> Feed ID: {}", name, feed_id);
                    
                //     // Create a tag with just a name
                //     let tag = tag::Tag::new(name);
                    
                //     let rule = FromFeed {
                //         tag,
                //         feed_id,
                //     };
                    
                //     feed_manager.tag_manager.add_rule(TagRuleEnum::FromFeed(rule));
                //     feed_manager.tag_manager.save_to_file()?;
                // }
                
                Some(RuleCommands::Apply) => {
                    info!("Applying rules to existing items...");
                    feed_manager.apply_rules_to_existing_items().await?;
                    info!("Rules applied successfully.");
                }
                
                None => {
                    warn!("Please specify a rule command. Use --help for options.");
                }
            }
        }
        
        Some(Commands::Folders { subcommand }) => {
            match subcommand {
                Some(FolderCommands::List) => {
                    info!("Listing folders:");
                    if folder_manager.folders.is_empty() {
                        info!("No folders defined");
                    } else {
                        for (i, folder) in folder_manager.folders.iter().enumerate() {
                            info!("Folder {}: {}", i + 1, folder.name);
                        }
                    }
                },
                
                Some(FolderCommands::Reload) => {
                    info!("Reloading folder configuration from: {}", "folders.yml");
                    match folder_manager.reload_config() {
                        Ok(_) => {
                            info!("Folder configuration reloaded successfully");
                        },
                        Err(e) => {
                            error!("Failed to reload folder configuration: {}", e);
                        }
                    }
                },
                
                None => {
                    warn!("Please specify a folder command. Use --help for options.");
                },
            }
        },
        
        None => {
            warn!("Please specify a command. Use --help for options.");
        }
    }
    
    info!("TagRss finished");
    Ok(())
}
