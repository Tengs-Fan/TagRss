use std::io;
use log::{LevelFilter, Level};
use fern::colors::{ColoredLevelConfig, Color};
use anyhow::Result;

/// Log configuration options
pub struct LogConfig {
    /// Log level for console output
    pub console_level: LevelFilter,
    /// Log level for file output
    pub file_level: LevelFilter,
    /// Path to log file (None means no file logging)
    pub log_file: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            console_level: LevelFilter::Info,
            file_level: LevelFilter::Debug,
            log_file: None,
        }
    }
}

/// Initialize the logging system with the provided configuration
pub fn init(config: LogConfig) -> Result<()> {
    // Configure colors for log levels
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Green)
        .debug(Color::Blue)
        .trace(Color::Magenta);

    // Base dispatcher with formatting
    let base_config = fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{} [{}] [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.target(),
                colors.color(record.level()),
                message
            ))
        })
        .level(LevelFilter::Trace); // Collect everything initially, then filter at each output

    // Console logger
    let console_config = fern::Dispatch::new()
        .level(config.console_level)
        .chain(io::stdout());

    // Combine the configurations
    let mut log_config = base_config.chain(console_config);

    // Add file logger if configured
    if let Some(log_file) = config.log_file {
        let file_config = fern::Dispatch::new()
            .level(config.file_level)
            .chain(fern::log_file(log_file)?);
        
        log_config = log_config.chain(file_config);
    }

    // Apply the configuration
    log_config.apply()?;
    
    Ok(())
}

/// Utility function to convert a string to a log level
pub fn parse_log_level(level: &str) -> LevelFilter {
    match level.to_lowercase().as_str() {
        "off" => LevelFilter::Off,
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info, // Default to Info for unrecognized levels
    }
} 