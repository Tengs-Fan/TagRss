# TagRss

A tag-based RSS reader. Sources carry tags, folders are rules over tags, and the same article from several feeds is one entry. The design is in [DESIGN.md](DESIGN.md).

## Use

```sh
tagrss              # open the reader in the browser (local only, on 127.0.0.1)
tagrss sync         # fetch every source; good for cron or launchd
tagrss check        # validate the config after editing it
tagrss tui          # the same reader in the terminal
```

In the reader: `j`/`k` move, `Enter` opens in a new tab and marks read, `r` toggles read, `R` marks the folder read, `J`/`K` change folder, `s` syncs, `c` reloads the config, `/` filters, `?` shows the keys.
The Sources view lists every source with its last fetch error.

## Setup

```sh
cargo install --path .
tagrss import feeds.opml   # appends the OPML's feeds to the config
$EDITOR ~/.config/tagrss/tagrss.toml   # add [[folder]] rules; see examples/tagrss.toml
tagrss sync && tagrss
```

Files:

- Config: `~/.config/tagrss/tagrss.toml`, your whole setup; keep it in git.
- Database: `~/.local/share/tagrss/tagrss.db`, a disposable cache of fetched entries and read marks.

Both can be overridden with `--config` and `--db`.
