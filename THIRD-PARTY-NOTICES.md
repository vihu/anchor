# Third-party notices

## Schibsted Grotesk

`ui/fonts/SchibstedGrotesk.ttf` is the Schibsted Grotesk typeface by
Schibsted Media, under the SIL Open Font License 1.1; the licence is in
`ui/fonts/OFL.txt`.

## mpv

anchor plays every stream by running the mpv installed on the computer
(https://mpv.io). It neither links nor carries mpv, and none of the
release builds include it.

## Rust libraries

The Rust crates anchor is built from are listed in `Cargo.lock`, each under
its own licence; `cargo deny check licenses` (`deny.toml`) keeps every one
compatible with GPL-3.0-or-later.
