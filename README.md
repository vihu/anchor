# anchor

[![CI](https://github.com/vihu/anchor/actions/workflows/ci.yml/badge.svg)](https://github.com/vihu/anchor/actions/workflows/ci.yml)
[![Release](https://github.com/vihu/anchor/actions/workflows/release.yml/badge.svg)](https://github.com/vihu/anchor/actions/workflows/release.yml)

A native Stremio client for Linux and macOS. Sign in with your Stremio
account: anchor shows your library and the catalogs of your addons, and
plays every stream in your own `mpv`, so your `mpv.conf`, scripts (ModernZ,
say), shaders and Dolby Vision handling (`vo=gpu-next`) apply untouched.

## Design

- anchor draws no video. Each stream opens in the mpv installed on your
  computer, which anchor follows over mpv's JSON IPC socket: where you
  are, and how it ended.
- Your Stremio account is the source of truth: the addons in its order,
  and its library. Progress goes back to it as you watch (every 30 s and
  when mpv closes), with Stremio's own rules, so Continue watching matches
  on your TV and phone.
- Stream addons' answers show as they wrote them (AIOStreams' formatting,
  emoji included), in their order. anchor ranks and filters nothing.
- Only direct links play (debrid, TorBox, HTTP). Torrents are counted and
  left out.
- The UI is [Slint]. Posters are decoded off the UI thread and only those
  near the screen are kept.
- The login key lives in the system keychain (Secret Service on Linux,
  Keychain on macOS), never in a file or a log. Addon and stream URLs,
  which carry keys, are never logged or shown.

Status: 0.1.0, not released yet (changes in `CHANGELOG.md`).

## What it does

| Area     | Supported                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------- |
| Home     | The title to resume, Continue watching from your library, a row per catalog                       |
| Sidebar  | Every catalog your addons offer, under Movies, Series and any other type, each section folds away |
| Catalogs | Full width, by genre, the next page as you scroll                                                 |
| Search   | Every catalog that can search, as you type, and a full results screen                             |
| Titles   | Movie and series pages: seasons, episodes, watched marks, the next episode selected, Mark watched |
| Streams  | Every stream addon's answer, the stream you played last selected, why one failed                  |
| Playing  | Your mpv, at the resume point, with addon subtitles in your languages; a now-playing bar          |
| Settings | Account and Sync now, your addons (read-only), mpv and its arguments, subtitle languages, About   |

anchor brings no content of its own: it plays what your addons offer.
Install, remove and configure addons on the Stremio website.

## Install

anchor needs mpv: `pacman -S mpv`, `apt install mpv`, `brew install mpv`.
On macOS it also looks in Homebrew's prefixes and `/Applications/mpv.app`,
since apps started from Finder do not see your shell's `PATH`; Settings >
Player takes any other path.

From [Releases](https://github.com/vihu/anchor/releases):

- Linux (x86_64 and arm64): `anchor-<version>-<arch>.AppImage`; `chmod +x`
  it and run it.
- macOS, Apple silicon and Intel: `anchor-<version>-macos-universal.zip`.
  Unzip it and move `anchor.app` to Applications. It is not notarized:
  open it once, then allow it in System Settings > Privacy & Security.

## Building

```bash
cargo run --release
```

The quality gate, as CI runs it:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check licenses
```

`cargo test -- --ignored` also runs the tests that need the network (the
addon and API clients against Cinemeta, OpenSubtitles and the Stremio API,
without an account) and the one that needs mpv.

## Without an account

`tools/fake-stremio.py` serves a stand-in Stremio API, a catalog addon and
a stream addon, with two titles left unfinished:

```bash
python3 tools/fake-stremio.py --media some-video.mp4
ANCHOR_API_URL=http://127.0.0.1:8099/api/ cargo run --release
```

Sign in with `demo@example.com` and `demo`. `--posters DIR` takes
`poster-*.jpg` and `still-*.jpg` for the artwork. Signing in stores the
fake key in the system keychain; remove it with
`secret-tool clear service anchor username fake-user`.

The snapshot tour clicks through every screen without a display and saves
each as a PNG:

```bash
ANCHOR_API_URL=http://127.0.0.1:8099/api/ cargo run --features snapshots -- --snapshots target/snapshots
```

## Licence

GPL-3.0-or-later. Schibsted Grotesk is under the SIL Open Font License
1.1 (`ui/fonts/OFL.txt`); see `THIRD-PARTY-NOTICES.md`. anchor is an
independent client for the Stremio addon protocol, not made by Stremio.

[Slint]: https://slint.dev
