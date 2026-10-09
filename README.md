# fyi

**fyi** (*for your information*) tells you about your files. Its command is
**`lsi`**: `ls` plus *info*, and if you forget it, typing `ls` and Tab finds it.

by **S.G.Johansson** · <s.johansson.it@gmail.com> · <https://voidflow.tech/>

A colorful, WSL2-aware `ls` and `tree` replacement for Linux. Every listing shows
file sizes color-graded from cool to hot and names colored by what **you** are
actually allowed to do with them. Output is a compact multi-column grid, a single
column, or a clean tree. Windows paths just work on WSL, and recent listings can be
replayed from history.

One static Rust binary, no runtime dependencies.

## Why

Nine times out of ten I just want to look around my directories: what is in here,
how big is it, and can I touch it. Plain `ls` makes me work for that. Sizes need
`-s` *and* `-h` before a human can read them, colors need `--color`, sorting needs
`--sort=…`, and permissions come as `-rwxr-x---` to decode in your head. On WSL,
every Windows path goes through `wslpath` first.

lsi is the listing I wanted to just type and read, with no flags for the everyday
case:

- **Sizes are always there**, readable at a glance by color, from cool blue for
  small files to hot orange for huge ones.
- **Permissions are a color on the name**, judged for *you*, not a mode string.
- **The layout fits your terminal**. Long names are shortened, never wrapped.
- **Windows paths just work** on WSL, even pasted unquoted.
- **What you just looked at is kept**, so you can get it back with `lsi -b`.

Flags exist for the uncommon cases, not the common one.

## Install

`install.sh` downloads the prebuilt static binary for your machine (x86_64 or
aarch64), checks it against the published SHA-256 and installs it as `lsi`.

```sh
wget2 https://raw.githubusercontent.com/SGJohansson/FYI/main/install.sh   # or curl -fsSLO
chmod +x install.sh
./install.sh          # for you:       ~/.local/bin/lsi
sudo ./install.sh     # system-wide:   /usr/local/bin/lsi
```

| Option   | Does                                                                   |
|----------|------------------------------------------------------------------------|
| *(none)* | install for this user into `~/.local/bin` (`$XDG_BIN_HOME` if set)     |
| `-s`     | install system-wide into `/usr/local/bin`; needs root, implied by sudo |
| `-u`     | uninstall: removes `lsi` only if this script installed it, unchanged   |
| `-f`     | replace an existing `lsi` (or remove a changed one with `-u`) without asking |
| `-y`     | answer yes to questions                                                |
| `-v TAG` | install a given release, e.g. `-v v0.3.0`                              |
| `-l DIR` | install from a directory holding `fyi-<arch>-linux-musl` and its `.sha256`: no download, for offline or LAN machines |

What it does and does not do:

- **Creates missing directories without asking**: `~/.local/bin`, the log directory.
- **Never overwrites silently.** An existing, different `lsi` is only replaced
  after you answer `y` (or with `-f`). Without a terminal it stops instead.
- **Moves and deletes nothing else.** Older `fyi`/`lsi` copies elsewhere in your
  PATH are listed, not touched. Your shell profile is not edited.
- **Logs every action**, one line each, to `~/.local/state/fyi/install.log`
  (user) or `/var/log/fyi-install.log` (root). `-u` uses this log to recognise
  what it installed.
- Falls back to building from source when no prebuilt binary fits and Rust is
  installed.

`curl -fsSL https://raw.githubusercontent.com/SGJohansson/FYI/main/install.sh | sh`
works too. Or grab `fyi-<arch>-linux-musl` from the
[latest release](https://github.com/SGJohansson/FYI/releases/latest) yourself and
install it as `lsi`.

From source:

```sh
cargo install --git https://github.com/SGJohansson/FYI
```

Fully static build from source:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
install -m 755 target/x86_64-unknown-linux-musl/release/lsi ~/.local/bin/
```

Optional: `alias ls=lsi`. When stdout is not a terminal, lsi prints bare names one
per line, so pipes and scripts keep working.

## Usage

```
lsi [OPTIONS] [PATHS]...

  -a, --all            show hidden entries
  -1, --single         one entry per line
  -f, --full-names     never shorten long names
  -r, --tree           recursive tree view (also -R, --recursive)
  -L, --level N        max tree depth (implies --tree)
  -n, --limit N        max entries per directory in tree view [default: 200, 0 = none]
  -d, --dir-sizes      compute directory sizes (walks subtrees; slow on /mnt/*)
      --ext LIST       list only entries matching LIST (see Filtering)
      --eext LIST      hide entries matching LIST (wins over --ext)
      --legend         filter keywords, examples and the color legend
  -p, --wslpath PATH…  convert a path Windows ⇄ WSL and print it (see Paths)
      --paste          convert the path(s) in the Windows clipboard
      --copy           also put the converted path in the Windows clipboard
      --to FORMAT      auto | linux | win | mixed  (for -p / --paste)
  -q, --quote          shell-quote the converted path
      --init SHELL     print the wcd / wcp shell functions (bash, zsh)
  -b, --back [N]       replay a previous listing (1 = most recent)
      --hist           list stored history
      --no-record      do not record this listing
      --hist-bytes B   max bytes per history entry [default: 1048576]
      --color WHEN     auto | always | never  (NO_COLOR is honored)
  -w, --width COLS     override terminal width
```

## Filtering

`--ext` lists only what matches; `--eext` hides what matches and wins over `--ext`.
Terms are comma-separated and case-insensitive; `a+b` must match both.

```sh
lsi --ext=dir,mp3,mkv          # directories, mp3 and mkv files
lsi --eext=sh,py,exe           # everything except those
lsi --ext=dir,bin,hidden       # directories, executables, dotfiles
lsi --ext=mkv+large            # only big videos
lsi --eext=small               # hide everything under 1M
lsi -r --ext=large             # tree of where the gigabytes are
```

| Term                         | Matches                                   |
|------------------------------|-------------------------------------------|
| `dir`                        | directories (incl. links to directories)  |
| `file` `link` `broken`       | regular files, symlinks, broken symlinks  |
| `bin` (`exe`)                | executable files                          |
| `hidden`                     | dotfiles (`--ext=hidden` implies `-a`)    |
| `ro` `locked`                | not writable / not readable by you        |
| `suid` `empty`               | setuid/setgid, zero-byte files            |
| `small` `mid` `large`        | < 1M, 1M – 1G, ≥ 1G                       |
| `s1` … `l3`                  | one size band (see [Colors](#colors))     |
| `mp3`, `tar.gz`              | file extension (any other word is one too)|
| `.bin`                       | extension, even when it is a keyword      |

No wildcards: `mp3` matches `name.mp3` only, and terms containing `*`, `?` or `/`
are rejected. Keywords win over extensions: `bin` means executables, `.bin` means
files ending in `.bin`.

- Flat view shows directories only when `dir` is in `--ext`.
- Tree view keeps directories that still hold matches; with `dir` it keeps them all.
- `-n` counts what survives the filter; the header shows how many were filtered.
- Size terms see directories only with `-d`.
- Filters also apply to replays: `lsi -b --ext=mp3`.

`lsi --legend` prints all of this, with more examples and the color legend, in your
terminal.

## Colors

### Size (always shown, binary units)

| Key | Size          | Color                 |
|-----|---------------|-----------------------|
| s1  | < 1K          | light blue            |
| s2  | 1K – 100K     | blue-green            |
| s3  | 100K – 1M     | light blue-green      |
| m1  | 1M – 10M      | blue                  |
| m2  | 10M – 100M    | purple                |
| m3  | 100M – 1G     | pink                  |
| l1  | 1G – 10G      | red                   |
| l2  | 10G – 1T      | orange                |
| l3  | ≥ 1T          | **bold orange**       |

`small` = s1–s3, `mid` = m1–m3, `large` = l1–l3. All work with `--ext` / `--eext`.

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
lsi 'C:\Users\me'                 # quoted
lsi C:/Users/me                   # forward slashes
lsi C:\Users\me                   # unquoted: bash delivers "C:Usersme"
lsi C:\Program Files\Git          # unquoted with spaces: split args are rejoined
lsi '\\wsl.localhost\Ubuntu\home'  # this distro's UNC path
```

The unquoted form arrives with its backslashes removed by the shell. fyi recovers it
by matching the text against the names that actually exist on disk. If more than one
path matches, it lists the candidates rather than guessing. A path that exists
literally is always used as-is.

Listings under a drive mount show their Windows spelling next to the path
(`/mnt/c/Users  ⇄ C:\Users`).

## Paths: Windows ⇄ WSL

`wslpath` gives up as soon as the shell has touched a path. fyi does not:
everything after `-p` (or `--wslpath`) is the path, quoted or not, with spaces or
not. The backslashes bash removed are recovered against what exists on disk, the
same way listings do it.

```sh
$ lsi -p D:\My Files\Graphics\Designs        # unquoted, straight from Explorer
/mnt/d/My Files/Graphics/Designs
$ cd "$(lsi -p D:\My Files\Graphics\Designs)"
$ lsi -p                                       # here, the other way
D:\My Files\Graphics\Designs
$ lsi -p /home/me
\\wsl.localhost\Ubuntu\home\me
$ lsi -q -p D:\Kalles filer\vad e detta.mp3   # quoted for pasting
'/mnt/d/Kalles filer/vad e detta.mp3'
```

- The direction is automatic: a Windows path becomes a Linux path and the other way
  round. `--to linux|win|mixed` forces one; `--to win` on a mangled Windows path
  gives its exact spelling on disk.
- Output is one bare line per path: right for `"$(…)"`, `while read` and
  `xargs -d '\n'`. `-q` shell-quotes it.
- Other options go before `-p`. `lsi -p -` reads paths from stdin, one per line.
- A quoted or pasted path converts even when it does not exist yet, like `wslpath`.
  An unquoted one has lost its backslashes and can only be recovered if it exists.

### The clipboard

`--paste` reads the Windows clipboard: a path from Explorer's *Copy as path*
(Shift + right-click, quotes are stripped) or files copied with Ctrl+C. The shell
never sees the text, so names with `(`, `&` or `'` are fine. `--copy` puts the result
back in the clipboard, ready for Explorer's address bar.

```sh
cd "$(lsi --paste)"
lsi --copy -p .            # K:\VFSH\omfile in the clipboard
```

### wcd and wcp

`cd $(…)` can never work unquoted with spaces: the shell splits the result again.
A program also cannot change your shell's directory. So fyi ships two small shell
functions:

```sh
# ~/.bashrc  (or ~/.zshrc with zsh)
eval "$(lsi --init bash)"
```

```sh
wcd D:\My Files\Graphics\Designs   # cd, unquoted, no $( )
wcd                                  # cd to the path in the clipboard
wcd D:\Music\song.mp3               # a file: cd to its folder
wcp                                  # this folder → clipboard as D:\…
wcp ~/notes.txt                      # \\wsl.localhost\… → paste in Windows
```

## History

The last 50 listings are stored in full under `$XDG_STATE_HOME/fyi/history`
(default `~/.local/state/fyi/history`). Entries hold the scanned data, not rendered
output, so a replay is laid out for your current terminal.

- `lsi --hist` lists the stored entries; `lsi -b 3` replays entry 3.
- Each entry is capped at `--hist-bytes`. A larger listing is pruned, deepest
  levels first, and marked as pruned.
- Listing the same place twice in a row replaces the previous entry.
- Only interactive listings are recorded. Piped output never is.

## Author

**S.G.Johansson**: <s.johansson.it@gmail.com> · <https://voidflow.tech/>

Bug reports and ideas are welcome as
[issues](https://github.com/SGJohansson/FYI/issues).

## License

Copyright © 2026 S.G.Johansson. Licensed under the
[Apache License, Version 2.0](LICENSE).
