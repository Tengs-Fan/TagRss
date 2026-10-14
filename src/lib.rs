//! TagRss's core. User interfaces use only what is exported here.

mod config;
mod core;
mod db;
mod entry;
mod feed;
mod opml;
mod rule;
mod tags;

pub use crate::config::{default_config_path, default_db_path, Config, Folder, Loaded, Source};
pub use crate::core::{check, import_opml, Core, FolderSummary, SourceStatus, SyncSummary, Syncer};
pub use crate::entry::Entry;
pub use crate::feed::SyncEvent;
pub use crate::opml::ImportReport;
pub use crate::rule::Expr;
