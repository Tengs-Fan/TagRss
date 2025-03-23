use clap::{Parser, Subcommand};
use anyhow::Result;

mod db;
mod feed;
mod models;
mod tag;

use tag::{TagManager, TagRuleEnum, Contains, TimeRange, FromFeed};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
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
    
    /// Add a new from-feed rule
    #[command(name = "add-feed")]
    AddFromFeed {
        /// Name of the tag
        name: String,
        
        /// ID of the feed
        feed_id: i64,
    },
    
    /// Apply rules to existing items
    #[command(name = "apply")]
    Apply,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize database
    let db = db::Database::new("sqlite:tagrss.db").await?;
    let mut feed_manager = feed::FeedManager::new(db);
    
    // Initialize tag manager
    let mut tag_manager = TagManager::new("tag_rules.json");
    
    match args.command {
        Some(Commands::AddFeed { url }) => {
            println!("Adding feed: {}", url);
            feed_manager.add_feed(&url).await?;
        }
        
        Some(Commands::ListFeeds) => {
            println!("Listing feeds:");
            let feeds = feed_manager.db.get_feeds().await?;
            for (id, url, title) in feeds {
                let title = title.unwrap_or_else(|| "no title".to_string());
                println!("ID: {}, URL: {}, Title: {}", id, url, title);
            }
        }
        
        Some(Commands::UpdateFeeds) => {
            println!("Updating feeds:");
            feed_manager.update_feeds(Some(&tag_manager)).await?;
            println!("Feeds updated successfully.");
        }
        
        Some(Commands::Rules { subcommand }) => {
            match subcommand {
                Some(RuleCommands::List) => {
                    println!("Listing rules:");
                    for (i, rule) in tag_manager.rules().iter().enumerate() {
                        println!("Rule {}: {:?}", i + 1, rule);
                    }
                }
                
                Some(RuleCommands::AddContains { name, target, case_sensitive }) => {
                    println!("Adding contains rule: {} -> {}", name, target);
                    
                    // First, ensure the tag exists in the database
                    let tag_id = match feed_manager.db.get_tag_by_name(&name).await? {
                        Some(tag) => tag.id,
                        None => feed_manager.db.add_tag(&name).await?,
                    };
                    
                    let rule = Contains {
                        name: name.clone(),
                        tag_id,
                        target_string: target,
                        case_sensitive,
                    };
                    
                    tag_manager.add_rule(TagRuleEnum::Contains(rule));
                    tag_manager.save_to_file()?;
                }
                
                Some(RuleCommands::AddTimeRange { name, start, end }) => {
                    println!("Adding time range rule: {}", name);
                    
                    // Parse dates if provided
                    let start_date = if let Some(start) = start {
                        Some(chrono::DateTime::parse_from_rfc3339(&start)?.with_timezone(&chrono::Utc))
                    } else {
                        None
                    };
                    
                    let end_date = if let Some(end) = end {
                        Some(chrono::DateTime::parse_from_rfc3339(&end)?.with_timezone(&chrono::Utc))
                    } else {
                        None
                    };
                    
                    // First, ensure the tag exists in the database
                    let tag_id = match feed_manager.db.get_tag_by_name(&name).await? {
                        Some(tag) => tag.id,
                        None => feed_manager.db.add_tag(&name).await?,
                    };
                    
                    let rule = TimeRange {
                        name: name.clone(),
                        tag_id,
                        start: start_date,
                        end: end_date,
                    };
                    
                    tag_manager.add_rule(TagRuleEnum::TimeRange(rule));
                    tag_manager.save_to_file()?;
                }
                
                Some(RuleCommands::AddFromFeed { name, feed_id }) => {
                    println!("Adding from-feed rule: {} -> Feed ID: {}", name, feed_id);
                    
                    // First, ensure the tag exists in the database
                    let tag_id = match feed_manager.db.get_tag_by_name(&name).await? {
                        Some(tag) => tag.id,
                        None => feed_manager.db.add_tag(&name).await?,
                    };
                    
                    let rule = FromFeed {
                        name: name.clone(),
                        tag_id,
                        feed_id,
                    };
                    
                    tag_manager.add_rule(TagRuleEnum::FromFeed(rule));
                    tag_manager.save_to_file()?;
                }
                
                Some(RuleCommands::Apply) => {
                    println!("Applying rules to existing items...");
                    feed_manager.apply_rules_to_existing_items(&tag_manager).await?;
                    println!("Rules applied successfully.");
                }
                
                None => {
                    println!("Please specify a rule command. Use --help for options.");
                }
            }
        }
        
        None => {
            println!("Please specify a command. Use --help for options.");
        }
    }

    Ok(())
}
