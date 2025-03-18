use clap::Parser;
use anyhow::Result;

mod db;
mod feed;
mod models;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    add: Option<String>,

    #[arg(short, long)]
    list: bool,

    #[arg(short, long)]
    update: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize database
    let db = db::Database::new().await?;
    let feed_manager = feed::FeedManager::new(db);
    
    match args.add {
        Some(url) => {
            println!("Adding feed: {}", url);
            feed_manager.add_feed(&url).await?;
            println!("Feed added successfully");
        }
        None => {}
    }

    if args.list {
        println!("Listing feeds:");
        let feeds = feed_manager.get_feeds().await?;
        for (id, url, title) in feeds {
            println!("ID: {}, Title: {}, URL: {}", id, title.unwrap_or_else(|| "No title".to_string()), url);
        }
    }

    if args.update {
        println!("Updating feeds:");
        feed_manager.update_feeds().await?;
        println!("Feeds updated successfully");
    }

    Ok(())
}
