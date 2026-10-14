# TagRss Design

## Principle

TagRss is defined by its contract with the user: what the user gives the system, and what the system shows back.
Everything else is an internal detail and must not leak into that contract.

The user gives the system **one file** containing the sources (with tags) and the folders (with rules over tags).
That file is the user's whole setup: keep it in git, copy it to a new machine, and the reader is fully restored.

The system shows back **folders**, each a list of entries (title, source, date, link), and accepts one action: **mark read** / **mark unread**.
Articles are read in the browser; TagRss links out and does not render content.

## The config file

One TOML file, `tagrss.toml`, with two lists.

```toml
[[source]]
url  = "https://www.youtube.com/feeds/videos.xml?channel_id=..."
tags = ["tech/semi", "format/video"]

[[source]]
url  = "https://alexandreafonso.me/feed/"
tags = ["politics"]

[[folder]]
name = "Semi deep dives"
rule = "tech/semi AND length/long AND NOT state/read"

[[folder]]
name = "Today"
rule = "age/today AND NOT state/read"
```

- `source.url` is the feed URL and the source's identity.
- `source.title` is optional; when absent the feed's own title is shown.
- `source.tags` is optional; a source without tags still gets system tags.
- `folder.name` must be unique; `folder.rule` is a rule expression (below).

The file is validated as a whole on load.
A bad rule, a duplicate folder name, a duplicate source URL, or a user tag in a reserved namespace is an error that names the offending entry; nothing is silently skipped.

TOML rather than YAML: the file is two flat lists, which TOML handles well; TOML has no implicit typing or significant indentation; and the `toml` crate is well maintained while `serde_yaml` is not.

OPML is not a source of truth.
It exists only as `import` (OPML → append `[[source]]` entries to the file) and `export` (file → OPML) for moving between readers.

## Tags

A tag is a path of segments separated by `/`, such as `tech/semi`.
A tag matches itself and everything below it: the rule `tech` matches `tech`, `tech/semi`, and `tech/semi/tsmc`, but not `fintech` or `topic/tech`.

An entry's tags are the union of its source's tags and the system tags.
Tags are **computed every time a folder is shown**, never stored, so they cannot go stale.

### System tags

The system provides tags in four reserved namespaces.
User tags must not start with `source/`, `age/`, `length/`, or `state/`; the file is rejected if one does.

| Tag | Meaning |
|---|---|
| `source/<slug>` | The entry's source; the slug is derived from the source title |
| `age/today` | Published within the last 24 hours |
| `age/week` | Published within the last 7 days |
| `age/month` | Published within the last 30 days |
| `length/short` | Under 500 words |
| `length/medium` | 500 to 2999 words |
| `length/long` | 3000 words or more |
| `length/unknown` | The feed gave no text to count, which is typical of video feeds |
| `state/read` | Marked read |
| `state/unread` | Not marked read |

Age tags overlap on purpose: an entry from today also has `age/week` and `age/month`, so the rule `age/week` means "within the last week".
An entry without a publish date uses the time TagRss first saw it.

The thresholds above are fixed defaults for v1.
Making them configurable would mean a third section in the file, and is deferred until a real need appears.

## Rules

A folder rule is a boolean expression over tags.

```
rule   := or
or     := and ("OR" and)*
and    := not ("AND" not)*
not    := "NOT" not | atom
atom   := TAG | "(" or ")"
```

`NOT` binds tightest, then `AND`, then `OR`; keywords are case-insensitive; parentheses group.
Example: `(tech/semi OR tech/vr) AND NOT length/short AND state/unread`.

Rules only reference tags.
There are no comparisons such as `words > 3000` inside rules; thresholds belong to the system tags, so a rule stays readable and each threshold is defined once.

## Storage

The database is the system's cache, never the user's truth, and is never exposed.
It holds two things the system cannot get back on demand:

1. **Entry metadata**, because feeds forget: a feed shows only its latest entries, so anything older exists only in this cache.
2. **Read marks**, because "mark read" is a user action that is deliberately not in the config file, which would otherwise grow with every entry.

```
entries (
  id             INTEGER PRIMARY KEY,  -- short handle for the CLI; not stable across a rebuild
  source_url     TEXT NOT NULL,        -- the source's identity, from the config file
  entry_key      TEXT NOT NULL,        -- feed entry id, falling back to the link
  url            TEXT,                 -- link to open in the browser
  title          TEXT NOT NULL,
  published_at   TEXT,                 -- RFC 3339; feed "published", falling back to "updated"
  first_seen_at  TEXT NOT NULL,        -- when TagRss first fetched it
  word_count     INTEGER,              -- NULL when the feed gave no text
  UNIQUE (source_url, entry_key)
)

reads (
  entry_id  INTEGER PRIMARY KEY REFERENCES entries(id),
  read_at   TEXT NOT NULL
)

fetches (
  source_url  TEXT PRIMARY KEY,
  feed_title  TEXT,                    -- the feed's own title, shown when the config has none
  fetched_at  TEXT NOT NULL,
  error       TEXT                     -- last fetch error, NULL on success
)

meta (
  key    TEXT PRIMARY KEY,             -- currently only "rebuilt_at"
  value  TEXT NOT NULL
)
```

Entries are written once and never rewritten.
Mark read inserts a row into `reads`; mark unread deletes it.
There is no `sources` table, no `rules` table, and no tag column anywhere.

`fetches` is a cache of the last sync per source, rewritten by every sync and never a source of truth.
It exists because the feed's own title has to live somewhere once fetched, and because a sync over hundreds of feeds needs a place to report which ones failed.
`import` writes the OPML title into each `[[source]]`, so imported sources never depend on this cache for their names.

Entry identity is `(source, entry id)`, not the URL alone, so the same article appearing in two feeds is kept in both.
The integer `id` exists only so commands like `read 42` can be typed.

Entries keep their `source_url` after the source is removed from the config file, so unsubscribing does not delete history.
Such entries have no source tags and appear only in folders whose rules match on system tags alone.

### Word count

Content is not stored, so `word_count` is computed once at fetch time and cannot be recomputed later.
It counts words in the entry's content, falling back to its summary, after stripping HTML; plain text without any markup must count correctly.
Chinese and Japanese are written without spaces, so each Han, Hiragana, or Katakana character counts as one word; everything else is split on whitespace.
This makes a 3000-character Chinese article `length/long` rather than `length/short`; the thresholds may need tuning for Chinese once real counts are seen.
When neither content nor summary exists, `word_count` is NULL and the entry gets `length/unknown`.

### Losing the database

Losing the database loses history older than the feeds' current windows, and loses read marks.
The setup survives because it lives in the config file.

Recovery: the next sync refills entries from the feeds, and every entry published before the rebuild is treated as read.
This is recorded as a single `rebuilt_at` timestamp in the database, so no per-entry rows are needed.

A fresh install cannot be told apart from a rebuild, so `rebuilt_at` is set whenever the database file is created.
The first sync therefore starts at inbox zero: only entries published after installation show as unread, not the whole backlog of every feed.

## Locations

Paths never depend on the working directory.

- Config: `$XDG_CONFIG_HOME/tagrss/tagrss.toml` (default `~/.config/tagrss/tagrss.toml`), overridable with `--config`.
- Database: `$XDG_DATA_HOME/tagrss/tagrss.db` (default `~/.local/share/tagrss/tagrss.db`), created if missing.

## Commands

| Command | Does |
|---|---|
| `sync` | Fetch every source in the config file and add new entries |
| `folders` | List folders with their unread counts |
| `list <folder>` | List a folder's entries, newest first |
| `open <entry>` | Open the entry's link in the browser and mark it read |
| `read <entry>` / `unread <entry>` | Mark read or unread |
| `check` | Validate the config file and print the parsed rules |
| `import <opml>` / `export` | Move sources in from or out to OPML |

Editing sources and folders is done by editing the file, not through commands, so there is one source of truth.

## Non-goals for v1

- Reading articles inside TagRss; content is not stored.
- Tags derived from article content, such as regex rules on titles.
- Tagging individual entries by hand.

Each of these can be added later without changing the contract above: content storage is a new column, content rules and manual tags are new tag sources feeding the same rule language.

## Changes from the current code

Kept: the rule expression parser (`folder.rs`) and hierarchical tag matching (`Article::match_tag`).

Removed: `rules.yaml` and the `rules` table, the `sources` table as a source of truth, stored tag columns, `content` storage, the `rule` and `set-tags` commands, and `folders.yaml` / `feeds.opml` as separate files.

Fixed by design: word counts of plain text, entries without a link being dropped, the same URL in two feeds being dropped, cwd-relative paths, and age tags going stale.
