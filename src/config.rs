//! The user's config file: sources with tags, and folders with rules over tags.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::rule::Expr;
use crate::tags::{self, tag_matches, SYSTEM_TAGS};

#[derive(Debug, Clone)]
pub struct Source {
    pub url: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub name: String,
    pub rule: Expr,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub sources: Vec<Source>,
    pub folders: Vec<Folder>,
}

/// A valid config plus the problems that do not stop it from loading.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    source: Vec<RawSource>,
    #[serde(default)]
    folder: Vec<RawFolder>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    url: String,
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFolder {
    name: String,
    rule: String,
}

pub fn load(path: &Path) -> Result<Loaded> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config file {}", path.display()))?;
    parse(&text).with_context(|| format!("invalid config file {}", path.display()))
}

/// Parses and validates the whole file, reporting every error at once.
pub fn parse(text: &str) -> Result<Loaded> {
    let raw: RawConfig = toml::from_str(text)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let mut urls = HashSet::new();
    let mut sources = Vec::new();
    for (i, s) in raw.source.into_iter().enumerate() {
        let at = format!("source #{} ({})", i + 1, s.url);
        if s.url.trim().is_empty() {
            errors.push(format!("source #{}: url is empty", i + 1));
        } else if !urls.insert(s.url.clone()) {
            errors.push(format!("{at}: duplicate url"));
        }
        for tag in &s.tags {
            if let Err(e) = tags::validate(tag) {
                errors.push(format!("{at}: {e}"));
            } else if let Some(ns) = tags::reserved_namespace(tag) {
                errors.push(format!("{at}: tag \"{tag}\" uses the reserved prefix \"{ns}/\""));
            }
        }
        sources.push(Source { url: s.url, title: s.title, tags: s.tags });
    }

    let user_tags: HashSet<&str> = sources.iter().flat_map(|s| s.tags.iter().map(String::as_str)).collect();
    let mut names = HashSet::new();
    let mut folders = Vec::new();
    for (i, f) in raw.folder.into_iter().enumerate() {
        let at = format!("folder #{} ({})", i + 1, f.name);
        if f.name.trim().is_empty() {
            errors.push(format!("folder #{}: name is empty", i + 1));
        } else if !names.insert(f.name.clone()) {
            errors.push(format!("{at}: duplicate name"));
        }
        let rule = match Expr::parse(&f.rule) {
            Ok(rule) => rule,
            Err(e) => {
                errors.push(format!("{at}: rule \"{}\": {e}", f.rule));
                continue;
            }
        };
        for tag in rule.tags() {
            if let Err(e) = tags::validate(tag) {
                errors.push(format!("{at}: {e}"));
            } else if !matches_something(tag, &user_tags) {
                warnings.push(format!("{at}: tag \"{tag}\" matches no source tag and no system tag"));
            }
        }
        folders.push(Folder { name: f.name, rule });
    }

    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(Loaded { config: Config { sources, folders }, warnings })
}

/// Whether a rule tag can match anything: a system tag, any `source/...`, or some source's tag.
fn matches_something(query: &str, user_tags: &HashSet<&str>) -> bool {
    match tags::reserved_namespace(query) {
        Some("source") => true,
        Some(_) => SYSTEM_TAGS.iter().any(|t| tag_matches(t, query)),
        None => user_tags.iter().any(|t| tag_matches(t, query)),
    }
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback),
    }
}

pub fn default_config_path() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("tagrss").join("tagrss.toml")
}

pub fn default_db_path() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("tagrss").join("tagrss.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_is_clean() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/tagrss.toml");
        let loaded = load(&path).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.config.sources.len(), 14);
        assert_eq!(loaded.config.folders.len(), 10);
    }

    #[test]
    fn empty_file_is_valid() {
        let loaded = parse("").unwrap();
        assert!(loaded.config.sources.is_empty() && loaded.config.folders.is_empty());
    }

    #[test]
    fn reports_every_error_with_its_entry() {
        let text = r#"
            [[source]]
            url = "https://a.example/feed"
            tags = ["age/new", "ok/tag", "bad//tag"]

            [[source]]
            url = "https://a.example/feed"

            [[folder]]
            name = "One"
            rule = "ok AND"

            [[folder]]
            name = "One"
            rule = "ok"
        "#;
        let err = parse(text).unwrap_err().to_string();
        assert!(err.contains("source #1") && err.contains("reserved prefix \"age/\""), "{err}");
        assert!(err.contains("empty segment"), "{err}");
        assert!(err.contains("source #2") && err.contains("duplicate url"), "{err}");
        assert!(err.contains("folder #1 (One)") && err.contains("rule \"ok AND\""), "{err}");
        assert!(err.contains("folder #2 (One): duplicate name"), "{err}");
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(parse("[[source]]\nurl = \"x\"\ntag = [\"a\"]\n").is_err());
    }

    #[test]
    fn warns_about_tags_that_match_nothing() {
        let text = r#"
            [[source]]
            url = "https://a.example/feed"
            tags = ["topic/ai"]

            [[folder]]
            name = "F"
            rule = "topic AND topic/os AND age/yesterday AND age AND source/whatever"
        "#;
        let warnings = parse(text).unwrap().warnings;
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("\"topic/os\""));
        assert!(warnings[1].contains("\"age/yesterday\""));
    }
}
