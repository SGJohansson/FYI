# fyi

**fyi** — *for your information* — tells you about your files.

A colorful, WSL2-aware `ls` and `tree` replacement for Linux. Every listing shows
file sizes color-graded from cool to hot and names colored by what **you** are
actually allowed to do with them. Output is a compact multi-column grid, a single
column, or a clean tree. Windows paths just work on WSL, and recent listings can be
replayed from history.

One static Rust binary, no runtime dependencies.

## Install

```sh
cargo install --git https://github.com/SGJohansson/fyi
```

Fully static build (recommended for copying between machines):

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
install -m 755 target/x86_64-unknown-linux-musl/release/fyi ~/.local/bin/
```

Optional: `alias ls=fyi`. When stdout is not a terminal, fyi prints bare names one
per line, so pipes and scripts keep working.

## Usage

```
fyi [OPTIONS] [PATHS]...

  -a, --all            show hidden entries
  -1, --single         one entry per line
  -f, --full-names     never shorten long names
  -r, --tree           recursive tree view (also -R, --recursive)
  -L, --level N        max tree depth (implies --tree)
  -n, --limit N        max entries per directory in tree view [default: 200, 0 = none]
  -d, --dir-sizes      compute directory sizes (walks subtrees; slow on /mnt/*)
  -b, --back [N]       replay a previous listing (1 = most recent)
      --hist           list stored history
      --no-record      do not record this listing
      --hist-bytes B   max bytes per history entry [default: 1048576]
      --color WHEN     auto | always | never  (NO_COLOR is honored)
  -w, --width COLS     override terminal width
```

## Colors

### Size (always shown, binary units)

| Size          | Color                 |
|---------------|-----------------------|
| < 1K          | light blue            |
| 1K – 100K     | blue-green            |
| 100K – 1M     | light blue-green      |
| 1M – 10M      | blue                  |
| 10M – 100M    | purple                |
| 100M – 1G     | pink                  |
| 1G – 10G      | red                   |
| 10G – 1T      | orange                |
| ≥ 1T          | **bold orange**       |

### Name (effective access for the current user)

Checked with `faccessat(…, AT_EACCESS)`, so ACLs, group membership and root are
taken into account. First match wins:

| Entry                                   | Color          |
|-----------------------------------------|----------------|
| not readable (dir: not readable/enterable) | wine red    |
| directory, not writable                 | orange         |
| directory, full access                  | light yellow   |
| file, executable + writable             | light green    |
| file, executable                        | green          |
| file, not writable                      | muted red      |
| everything else                         | default        |

Symlinks are colored by their target and shown as `name → target`; broken links are
wine red and struck through. `ˢ`/`ᵍ` mark setuid/setgid.

## Tree view

```
~/proj                                    2 dirs · 3 files
├── Cargo.lock                     38K
├── Cargo.toml                    612B
├── run.sh → scripts/run.sh       200B
├── scripts/                              0 dirs · 1 file
│   └── run.sh                    200B
└── src/                                  1 dir · 2 files
    ├── main.rs                   4.2K
    ├── wsl.rs                    6.0K
    └── render/                           0 dirs · 2 files
        ├── color.rs              2.1K
        └── grid.rs               8.9K
```

- A directory's own files come before its subdirectories, so they stay next to it.
- Directories always end in `/` and carry a summary.
- Sizes share one right-aligned column regardless of depth.
- Guide lines are tinted per depth.
- Symlinked directories are never descended (no cycles).
- Oversized directories end with `… N more (x dirs, y files)`.

## WSL

On WSL, fyi converts Windows paths itself. It does not call `wslpath`, and it reads
the real drive mounts from `/proc/self/mountinfo`, so a custom `automount.root` works.

```sh
fyi 'C:\Users\me'                 # quoted
fyi C:/Users/me                   # forward slashes
fyi C:\Users\me                   # unquoted: bash delivers "C:Usersme"
fyi C:\Program Files\Git          # unquoted with spaces: split args are rejoined
fyi '\\wsl.localhost\Ubuntu\home'  # this distro's UNC path
```

The unquoted form arrives with its backslashes removed by the shell. fyi recovers it
by matching the text against the names that actually exist on disk. If more than one
path matches, it lists the candidates rather than guessing. A path that exists
literally is always used as-is.

Listings under a drive mount show their Windows spelling next to the path
(`/mnt/c/Users  ⇄ C:\Users`).

## History

The last 50 listings are stored in full under `$XDG_STATE_HOME/fyi/history`
(default `~/.local/state/fyi/history`). Entries hold the scanned data, not rendered
output, so a replay is laid out for your current terminal.

- `fyi --hist` lists the stored entries; `fyi -b 3` replays entry 3.
- Each entry is capped at `--hist-bytes`. A larger listing is pruned, deepest
  levels first, and marked as pruned.
- Listing the same place twice in a row replaces the previous entry.
- Only interactive listings are recorded. Piped output never is.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
