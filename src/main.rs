use clap::{Parser, Subcommand};
use anyhow::Result;
use log::{info, debug, warn, error};

mod db;
mod feed;
mod models;
mod tag;
mod logger;
mod folder;

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
    
    /// Manage folders
    #[command(name = "folders")]
    Folders {
        #[command(subcommand)]
        subcommand: Option<FolderCommands>,
    },
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
    
    // Initialize feed manager
    let feed_manager = feed::FeedManager::new(db);

    // Initialize folder manager with YAML config
    let mut folder_manager = FolderManager::new("folders.yml");
    
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
