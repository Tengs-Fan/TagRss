# TagRss

A tag-based RSS reader for the terminal.

You tag your **sources**; TagRss adds its own tags to every entry (age, length, read state, source); and **folders** are rules over those tags.
A folder is a saved question, such as "long reads from this week that I have not read", and not a place where entries are filed.

```toml
[[source]]
url  = "https://distill.pub/rss.xml"
tags = ["topic/ai", "depth/deep"]

[[source]]
url  = "https://www.youtube.com/feeds/videos.xml?channel_id=UCbfYPyITQ-7l4upoX8nvctg"
tags = ["topic/ai", "format/video"]

[[folder]]
name = "Today"
rule = "age/today AND state/unread"

[[folder]]
name = "AI, no videos"
rule = "topic/ai AND NOT format/video AND state/unread"
```

That one file is your whole setup. Keep it in git, copy it to another machine, and the reader is restored.
[DESIGN.md](DESIGN.md) explains the contract and why it is shaped this way.

## Install

```bash
cargo install --path .
```

## Getting started

Write a config file at `~/.config/tagrss/tagrss.toml`; [examples/tagrss.toml](examples/tagrss.toml) is a commented starting point.
Coming from another reader, import its OPML export instead, then add tags and folders by editing the file:

```bash
tagrss import feeds.opml
```

Then:

```bash
tagrss check
```

```bash
tagrss sync
```

```bash
tagrss folders
```

```bash
tagrss list Today
```

A new installation starts at inbox zero: the backlog of every feed counts as read, and only what is published from now on shows up as unread.

## Commands

| Command | Does |
|---|---|
| `tagrss sync` | Fetch every source and store new entries |
| `tagrss folders` | List folders with unread and total counts |
| `tagrss list <folder> [-n N]` | List a folder's entries, newest first; `*` marks unread |
| `tagrss open <id>` | Open an entry in the browser and mark it read |
| `tagrss read <id>...` / `tagrss unread <id>...` | Mark entries read or unread |
| `tagrss read --folder <folder>` | Mark everything in a folder read (`unread` works the same way) |
| `tagrss sources [--failed]` | List sources with their `source/` tag, entry count, and last sync |
| `tagrss check` | Validate the config file and show how each rule parses |
| `tagrss import <opml>` / `tagrss export` | Move sources in from or out to OPML |

Sources and folders are edited in the file, not through commands.
`--config <path>` and `--db <path>` override the default locations.

## Tags and rules

A tag is a path such as `topic/ai`. In a rule, a tag matches itself and everything below it, so `topic` matches `topic/ai` and `topic/os`.

Rules combine tags with `NOT`, `AND`, and `OR` (binding in that order) and parentheses:

```
(topic/electronics OR topic/os) AND NOT length/short AND state/unread
```

The system adds these tags to every entry; your own tags must not use these prefixes:

| Tag | Meaning |
|---|---|
| `source/<slug>` | The entry's source; `tagrss sources` lists the exact tags |
| `age/today`, `age/week`, `age/month` | Published within the last 24 hours, 7 days, 30 days |
| `length/short`, `length/medium`, `length/long` | Under 500 words, 500 to 2999, 3000 or more |
| `length/unknown` | The feed sent no full text: video feeds, and feeds that send only a summary |
| `state/read`, `state/unread` | Whether the entry is marked read |

Tags are computed each time a folder is shown, so they never go stale.

## Files

| | Default location |
|---|---|
| Config | `$XDG_CONFIG_HOME/tagrss/tagrss.toml` (`~/.config/tagrss/tagrss.toml`) |
| Database | `$XDG_DATA_HOME/tagrss/tagrss.db` (`~/.local/share/tagrss/tagrss.db`) |

The database is a cache of what the feeds said, plus your read marks.
Losing it loses the marks and any history older than what the feeds still publish; the next `sync` rebuilds the rest.

## Development

```bash
cargo test
```

```bash
cargo clippy --all-targets -- -D warnings
```

The tests need no network: the end-to-end tests in `tests/cli.rs` run the real binary against a feed server on localhost.
