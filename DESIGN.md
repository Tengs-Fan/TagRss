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
| `source/<slug>` | A source the entry came from, one tag per source; the slug is derived from the source title |
| `age/today` | Published within the last 24 hours |
| `age/week` | Published within the last 7 days |
| `age/month` | Published within the last 30 days |
| `length/short` | Under 500 words |
| `length/medium` | 500 to 2999 words |
| `length/long` | 3000 words or more |
| `length/unknown` | The feed gave no full text to count: video feeds, and feeds that send only a summary |
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

## Observations and entries

An **entry** is an article: the thing you read and mark read.
A **source** is only where TagRss heard about an entry, so one article reached through two feeds, such as a blog and an aggregator linking to it, is one entry.

What is stored is the **observation**: "feed F said item X, with this title, date, and link".
Observations are what the feeds said, one row per source, and are never rewritten.
Entries are computed from observations every time they are shown, like tags, so a better matching rule later regroups everything with nothing baked in.

### Grouping

Observations are grouped by their **normalized link**:

- the scheme becomes `https`, and the host is lowercased with a leading `www.` removed;
- the `#fragment` and a trailing `/` on the path are removed;
- known tracking parameters are removed: `utm_*`, `ref`, `ref_src`, `source`, `fbclid`, `gclid`, `mc_cid`, `mc_eid`;
- every other query parameter is kept, so `youtube.com/watch?v=a` and `youtube.com/watch?v=b` stay distinct.

Redirect links, such as feedburner's, are not followed; they may stay duplicates.

A normalized link groups observations only if **no single source has two observations with it**.
Some feeds give many items the same link, such as a homepage or a podcast's show page, and merging those would silently lose entries; if any source repeats a link, every observation with that link stays its own entry.
An observation without a link is always its own entry.

### Merging

When several observations form one entry:

| Field | Rule |
|---|---|
| Tags | The union of every observation's source tags, plus `source/<slug>` for each source |
| Date | The earliest publish date; observations without one are ignored, and if none has one, the earliest first-seen time is used |
| Title and link | From the observation that gave the date, ties broken by the smallest id |
| Word count | The largest known count, since one feed's full text beats another's teaser |
| Read | The most recent mark on any of its observations |

An entry's **id** is the smallest id among its observations.
Observations are never deleted and ids only grow, so an entry's id never changes when a later sync adds another observation of it.
Commands accept the id of any of an entry's observations; marking an entry writes the mark to all of its observations.

## Storage

The database is the system's cache, never the user's truth, and is never exposed.
It holds two things the system cannot get back on demand:

1. **Observations**, because feeds forget: a feed shows only its latest items, so anything older exists only in this cache.
2. **Read marks**, because "mark read" is a user action that is deliberately not in the config file, which would otherwise grow with every entry.

```
observations (
  id             INTEGER PRIMARY KEY,  -- short handle for the CLI; not stable across a rebuild
  source_url     TEXT NOT NULL,        -- the source's identity, from the config file
  item_key       TEXT NOT NULL,        -- the feed's own id for the item, falling back to the link
  url            TEXT,                 -- link to open in the browser
  title          TEXT NOT NULL,
  published_at   TEXT,                 -- RFC 3339; feed "published", falling back to "updated"
  first_seen_at  TEXT NOT NULL,        -- when TagRss first fetched it
  word_count     INTEGER,              -- NULL when the feed gave no full text
  UNIQUE (source_url, item_key)
)

marks (
  observation_id  INTEGER PRIMARY KEY REFERENCES observations(id),
  read            INTEGER NOT NULL,    -- 1 marked read, 0 marked unread
  marked_at       TEXT NOT NULL
)

fetches (
  source_url  TEXT PRIMARY KEY,
  feed_title  TEXT,                    -- the feed's own title, shown when the config has none
  fetched_at  TEXT NOT NULL,
  error       TEXT                     -- last fetch error, NULL on success
)
```

Observations are written once and never rewritten.
Mark read and mark unread both write rows into `marks`, recording which one, and an entry with no mark is unread.
There is no `sources` table, no `rules` table, no `entries` table, and no tag column anywhere.

`fetches` is a cache of the last sync per source, rewritten by every sync and never a source of truth.
It exists because the feed's own title has to live somewhere once fetched, and because a sync over hundreds of feeds needs a place to report which ones failed.
`import` writes the OPML title into each `[[source]]`, so imported sources never depend on this cache for their names.

Observations keep their `source_url` after the source is removed from the config file, so unsubscribing does not delete history.
An entry whose sources are all gone from the config has no source tags and appears only in folders whose rules match on system tags alone.

The database records its schema version, and a database with a different version is dropped and recreated.
That is consistent with it being a cache: a schema change costs the same as losing the database.

### Word count

Content is not stored, so `word_count` is computed once at fetch time and cannot be recomputed later.
It counts words in the item's full content (RSS `content:encoded`, Atom `content`) after stripping HTML; plain text without any markup must count correctly.
A summary (RSS `description`, Atom `summary`) is not counted: in practice it is a teaser or a bare link, so counting it would tag a long Distill article `length/short`.
Feeds that send only a summary therefore get `length/unknown`, including the few whose summaries are the whole post.
Chinese and Japanese are written without spaces, so each Han, Hiragana, or Katakana character counts as one word; everything else is split on whitespace.
This makes a 3000-character Chinese article `length/long` rather than `length/short`; the thresholds may need tuning for Chinese once real counts are seen.
When there is no full content, `word_count` is NULL and the observation gets `length/unknown`, unless another observation of the same entry has a count.

### First sync and losing the database

Everything is unread until marked: a first sync shows the whole current window of every feed as unread, typically about 20 entries per feed.
Marking a folder read clears it in one step, and a folder with the rule `state` matches every entry, since every entry has either `state/read` or `state/unread`.

Losing the database loses history older than the feeds' current windows, and loses read marks.
The setup survives because it lives in the config file, and the next sync is simply a first sync again.

## Core behavior

The core implements this document and decides every behavior that any user interface must share.
It never prints and never launches anything; it returns values and reports progress through events.
A user interface decides only presentation and input.
The test for where something belongs: if a second user interface would have to copy it, it belongs in the core.

Config:

- An invalid config file never runs: opening reports every error at once.
- Reloading, which happens only when a user interface asks for it, keeps the old config if the new file is invalid and returns the errors.

Reading:

- Every query is computed fresh from observations, the config, and the current time, read once per query so one list is consistent; nothing is cached between queries.
- Entries are ordered newest first by their date, ties broken by id.
- Opening an entry marks it read and returns its link; the user interface opens the link.
- Nothing else is implicit: viewing or scrolling past an entry never marks it.
- Marking a folder read marks what it contains at that moment; entries that arrive later stay unread.

Sync:

- One failing feed never stops the others; feeds are fetched 8 at a time with a 30-second timeout, and each failure is recorded on its source.
- Syncing twice adds nothing, and an interrupted sync keeps what it stored.
- Sync never deletes or rewrites observations, and nothing is ever pruned.
- Sync reports an event per source as it finishes.
- Sync runs on its own database connection, so a user interface can keep reading while it runs; the database uses write-ahead logging and waits briefly for locks.

Failures:

- A config error or a database error stops the operation.
- A feed error is data: it is recorded on the source and reported, never fatal.
- No feed content, however malformed, can make the core panic.

## Locations

Paths never depend on the working directory.

- Config: `$XDG_CONFIG_HOME/tagrss/tagrss.toml` (default `~/.config/tagrss/tagrss.toml`), overridable with `--config`.
- Database: `$XDG_DATA_HOME/tagrss/tagrss.db` (default `~/.local/share/tagrss/tagrss.db`), created if missing, overridable with `--db`.

## Commands

Running `tagrss` with no command opens the terminal user interface, which is where reading happens.
The commands are a second user interface over the same core, split by role.

**Operations** are stable, because they get scripted, for example `sync` from launchd or cron:

| Command | Does |
|---|---|
| `sync` | Fetch every source in the config file and add new observations |
| `check` | Validate the config file and print how its rules parse |
| `import <opml>` | Append the OPML's feeds to the config file, keeping its comments |
| `export` | Print the config's sources as OPML |

Their names and exit codes are a promise: a non-zero exit means a fatal config or database error, while feed failures are reported and do not fail the command.
Data goes to stdout and diagnostics to stderr.

**Reading** commands are aids for debugging and scripting, with no stability promise:

| Command | Does |
|---|---|
| `folders` | List folders with their unread counts |
| `list <folder>` | List a folder's entries, newest first |
| `open <entry>` | Open the entry's link in the browser and mark it read |
| `read <entry>…` / `unread <entry>…` | Mark entries read or unread |
| `read --folder <folder>` | Mark everything in a folder read |

There is no machine-readable output yet; a `--json` flag would make the output a contract, and is added only when a script needs it.

Editing sources and folders is done by editing the file, not through commands, so there is one source of truth.

### Terminal user interface

Two panes: folders with unread counts on the left, the selected folder's entries on the right.

| Key | Does |
|---|---|
| `j` / `k`, arrows | Move within the focused pane |
| `Tab`, `h` / `l` | Switch pane |
| `Enter` | On an entry: open it in the browser and mark it read |
| `r` | Toggle the selected entry read or unread |
| `R` | Mark the selected folder read |
| `s` | Sync, with per-source progress in the status line |
| `c` | Reload the config file; an invalid file keeps the old config and shows the errors |
| `q` | Quit |

The entry list is a snapshot: marking an entry read does not remove it from an unread folder until the list is refreshed by changing folders, syncing, or reloading, so the cursor never jumps.

## Non-goals for v1

- Reading articles inside TagRss; content is not stored.
- Tags derived from article content, such as regex rules on titles.
- Tagging individual entries by hand.

Each of these can be added later without changing the contract above: content storage is a new column, content rules and manual tags are new tag sources feeding the same rule language.

## Changes from the current code

Kept: the rule expression parser, now `rule.rs` and rejecting trailing tokens such as `a b`; hierarchical tag matching, now `tags::tag_matches`; the OPML reader, now unescaping attribute values.

Removed: `rules.yaml` and the `rules` table, the `sources` table as a source of truth, stored tag columns, `content` storage, the `rule` and `set-tags` commands, and `folders.yaml` / `feeds.opml` as separate files.

Fixed by design: word counts of plain text, entries without a link being dropped, the same article in two feeds being dropped (now merged into one entry), cwd-relative paths, and age tags going stale.
