# TagRss

A tag-based RSS reader that runs on your own machine.

You write one file: your feeds, each with tags, and your folders, each a rule over tags.
TagRss fetches the feeds, works out which entries belong in which folder, and shows them in a local web reader.

- **Folders are rules, not containers.** `topic/ai AND length/long AND state/unread` is a folder; an entry can appear in several.
- **Tags are hierarchical.** A rule for `topic` matches `topic/ai` and `topic/os`.
- **The system adds tags of its own**, from an entry's age, length, source and read state.
- **One article is one entry**, even when several of your feeds link to it.
- **One file is your whole setup.** There is no settings screen; edit the file, by hand or with an agent, and reload.

The full contract is in [DESIGN.md](DESIGN.md).

## Quick start

You need a Rust toolchain.

```sh
cargo install --path .

mkdir -p ~/.config/tagrss
cp examples/tagrss.toml ~/.config/tagrss/tagrss.toml   # or: tagrss import feeds.opml

tagrss check    # validate the config
tagrss sync     # fetch every source
tagrss          # open the reader in the browser
```

On the first sync everything is unread.

## The config file

```toml
[[source]]
url   = "https://distill.pub/rss.xml"   # required; also the source's identity
title = "Distill"                       # optional; overrides the feed's own title
tags  = ["topic/ai", "depth/deep"]      # optional; every entry from this source gets them

[[folder]]
name = "Deep AI"
rule = "topic/ai AND depth/deep AND state/unread"

[[folder]]
name = "Long reads"
rule = "length/long AND NOT topic/news"

[[folder]]
name = "Hardware"
rule = "(topic/electronics OR topic/hardware) AND state/unread"
```

[examples/tagrss.toml](examples/tagrss.toml) is a commented, working example.

### Rules

A rule is a boolean expression over tags, with `AND`, `OR`, `NOT` and parentheses.
`NOT` binds tightest, then `AND`, then `OR`.
A tag matches itself and everything below it, by whole segments: `topic` matches `topic/ai` but not `topics`.

`tagrss check` prints each rule fully parenthesized, so you can see how it was read.

### System tags

Your own tags must not start with these prefixes.

| Tag | Meaning |
|---|---|
| `state/unread`, `state/read` | Read state |
| `age/today`, `age/week`, `age/month` | Published in the last 24 hours, 7 days, 30 days |
| `length/short`, `length/medium`, `length/long` | Under 500 words, 500 to 2999, 3000 or more |
| `length/unknown` | The feed sent no full text (video feeds, summary-only feeds) |
| `source/<slug>` | The source it came from; the slug comes from the title |

Tags are computed when entries are shown and never stored, so changing a source's tags or a folder's rule takes effect on reload, for old entries too.

## The reader

`tagrss` serves the reader on `127.0.0.1` only and opens it in your browser.
Use `--port 8080` for a fixed address and `--no-browser` to skip opening it.

- **Sidebar:** switch between **Folders** and **Sources**. A source shows everything fetched from it, whatever folders those entries fall in; "All sources" lists every source with its last fetch error.
- **Preview:** selecting an entry shows what the feed sent in a side pane and marks it read. YouTube and bilibili entries play there.
- **Rules on hover:** hover a folder to see its rule as a tree, or a source to see its tags.
- **Unread only** is the default; `u` shows read entries too.

| Key | Does |
|---|---|
| `j` / `k` | Next / previous entry: preview it and mark it read |
| `Enter`, `o` | Open the original in a new tab |
| `r` | Toggle read |
| `u` | Unread only, or read entries too |
| `R` | Mark the folder read |
| `J` / `K` | Next / previous folder, or source |
| `/` | Filter by title |
| `s` | Sync |
| `c` | Reload the config file |
| `?` | Show the keys |

After editing the config, press `c`.
An invalid file is rejected with its errors and the old config stays in use.

## Commands

```sh
tagrss                  # the web reader
tagrss tui              # a simpler reader in the terminal
tagrss sync             # fetch every source; suitable for cron or launchd
tagrss check            # validate the config and print how its rules parse
tagrss import <opml>    # append an OPML file's feeds to the config, keeping its comments
tagrss export           # print the config's sources as OPML
```

`sync`, `check`, `import` and `export` are stable and safe to script.
A failing feed is reported and recorded on its source, and does not fail the command.

There are also commands for scripting and debugging, with no stability promise: `folders`, `list <folder>`, `open <entry>`, `read <entry>…`, `read --folder <folder>`, `unread <entry>…`, `sources`.

## Files

| File | Default | What it is |
|---|---|---|
| Config | `~/.config/tagrss/tagrss.toml` | Your whole setup. Keep it in git. |
| Database | `~/.local/share/tagrss/tagrss.db` | A cache of fetched entries, plus read marks. |

Override them with `--config` and `--db`; `XDG_CONFIG_HOME` and `XDG_DATA_HOME` are honoured.

Deleting the database loses read marks and entries that have since dropped out of their feeds; the next sync brings back whatever the feeds still carry, all unread.
Nothing is ever pruned.

## Limits

- The preview shows only what the feed sent; a feed that sends a teaser previews as a teaser.
- The reader is for one person on one machine: it listens on loopback only and has no login.
- The terminal reader has no preview, source view or unread toggle.
