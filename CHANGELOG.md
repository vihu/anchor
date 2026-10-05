# Changelog

## 0.1.0 (unreleased)

The first release: a native Stremio client for Linux and macOS that plays
through your own mpv.

- Sign in with a Stremio account; the login key stays in the system
  keychain. The account's addons and library load from the cache at once
  and sync in the background.
- Home: the title to resume, Continue watching from the library, and a row
  per catalog of your addons, posters kept only near the screen.
- The sidebar lists every catalog your addons offer, under Movies, Series
  and any other type; a catalog opens full width, by genre, the next page
  as you scroll. Ctrl+Up and Ctrl+Down step through the catalogs.
- Search at the top of every screen, across every catalog that can search,
  with results as you type.
- Movie and series pages. A movie shows its cast (with photos from
  AIOMetadata), certification, director, writers, release date, country
  and awards; a series its seasons, episodes, watched marks (Stremio's
  watched bitfield), the next episode selected. Mark watched.
- Streams: every stream addon's answer as it wrote it, the stream played
  last selected, torrents left out, why mpv could not play one.
- Playing: your mpv at the resume point, with addon subtitles in the
  languages you pick; a now-playing bar; progress written back to your
  Stremio account with Stremio's rules (watched past 70 %, the next
  episode past 90 %), seeks apart.
- Settings: the account and Sync now, the addons, mpv's path and extra
  arguments, subtitle languages, About with Clear cache.
- Release builds: AppImage for x86_64 and arm64, and a universal macOS app.
