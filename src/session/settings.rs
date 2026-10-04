//! Settings: the account and Sync now, the addons as the account has them,
//! mpv and the addon subtitles, and About with the files and Clear cache.

use std::rc::Rc;

use anchor::addon::Addon;
use anchor::player;
use camino::Utf8Path;
use jiff::Timestamp;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{Session, spawn};
use crate::languages::{self, LANGUAGES};
use crate::text;
use crate::ui::{AddonItem, Fact, Screen, SettingsData, Shell};

/// Where the account's addons are managed.
const ADDONS_URL: &str = "https://web.stremio.com/#/addons";
/// Where anchor's source is.
const SOURCE_URL: &str = "https://github.com/vihu/anchor";
/// Bytes in a megabyte, for the cache's size.
const MEGABYTE: u64 = 1024 * 1024;

impl Session {
    /// Wires the Settings screen's controls.
    pub(super) fn wire_settings(app: &crate::ui::AppWindow) {
        use super::with_session;
        let data = app.global::<SettingsData>();
        data.set_version(env!("CARGO_PKG_VERSION").into());
        data.set_build(build().into());
        data.set_language_options(ModelRc::new(VecModel::from(
            LANGUAGES
                .iter()
                .map(|(_, name)| SharedString::from(*name))
                .collect::<Vec<_>>(),
        )));
        data.on_sync(|| with_session(|s| s.sync()));
        data.on_mpv_path_edited(|text| {
            with_session(|s| {
                s.settings.borrow_mut().mpv_path = text.trim().to_owned();
                s.save_settings();
            });
        });
        data.on_check_mpv(|| with_session(|s| s.check_mpv()));
        data.on_mpv_args_edited(|text| {
            with_session(|s| {
                s.settings.borrow_mut().mpv_args = text.to_string();
                s.save_settings();
            });
        });
        data.on_toggle_subtitles(|| {
            with_session(|s| {
                {
                    let mut settings = s.settings.borrow_mut();
                    settings.addon_subtitles = !settings.addon_subtitles;
                }
                s.save_settings();
                s.fill_player();
            });
        });
        data.on_add_language(|i| with_session(|s| s.add_language(i)));
        data.on_remove_language(|i| with_session(|s| s.remove_language(i)));
        data.on_open_addons_site(|| open(ADDONS_URL));
        data.on_open_source(|| open(SOURCE_URL));
        data.on_clear_cache(|| with_session(|s| s.clear_cache()));
    }

    /// Shows Settings, filled.
    pub(super) fn open_settings(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        self.light_catalog(-1, -1);
        app.set_back_label("Home".into());
        self.fill_settings();
        self.check_mpv();
        self.show(Screen::Settings);
    }

    /// Fills the account, addons, player and files.
    pub(super) fn fill_settings(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let data = app.global::<SettingsData>();
        let shell = app.global::<Shell>();
        data.set_avatar(shell.get_avatar());
        data.set_email(shell.get_email());
        let (facts, addons) = {
            let state = self.state.borrow();
            let in_library = state.library.iter().filter(|i| !i.removed).count();
            let unfinished = anchor::library::continue_watching(&state.library).len();
            let facts = vec![
                fact("Library", &plural(in_library, "title")),
                fact("Continue watching", &plural(unfinished, "title")),
                fact("Addons", &state.sources.addons().len().to_string()),
                fact(
                    "Last synced",
                    &state.synced_at.map_or_else(|| "not yet".to_owned(), ago),
                ),
            ];
            let addons: Vec<AddonItem> = state.sources.addons().iter().map(addon_item).collect();
            (facts, addons)
        };
        data.set_facts(ModelRc::new(VecModel::from(facts)));
        data.set_addons(ModelRc::new(VecModel::from(addons)));
        self.fill_player();
        self.fill_files();
    }
}

// Private
impl Session {
    fn fill_player(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let data = app.global::<SettingsData>();
        let settings = self.settings.borrow();
        if data.get_mpv_path() != settings.mpv_path.as_str() {
            data.set_mpv_path(settings.mpv_path.as_str().into());
        }
        if data.get_mpv_args() != settings.mpv_args.as_str() {
            data.set_mpv_args(settings.mpv_args.as_str().into());
        }
        data.set_addon_subtitles(settings.addon_subtitles);
        data.set_languages(ModelRc::new(VecModel::from(
            settings
                .subtitle_languages
                .iter()
                .map(|code| SharedString::from(languages::name(code)))
                .collect::<Vec<_>>(),
        )));
    }

    /// Looks for mpv where Settings says, and says what it found.
    fn check_mpv(&self) {
        let configured = self.settings.borrow().mpv_path.clone();
        spawn(
            move || {
                let path = player::find_mpv(
                    Some(Utf8Path::new(&configured)).filter(|p| !p.as_str().is_empty()),
                );
                let version = path.as_deref().and_then(player::version);
                (configured, path, version)
            },
            |s, (configured, path, version)| {
                let Some(app) = s.app.upgrade() else {
                    return;
                };
                let data = app.global::<SettingsData>();
                let status = match (&path, version) {
                    (Some(path), Some(version)) if configured.is_empty() => {
                        format!("Found on your PATH: mpv {version}, at {path}.")
                    }
                    (Some(path), Some(version)) => format!("mpv {version}, at {path}."),
                    (Some(path), None) => format!("{path} does not run as mpv."),
                    (None, _) if configured.is_empty() => {
                        "Not found on your PATH. Install mpv (pacman -S mpv, brew install mpv), or say where it is.".to_owned()
                    }
                    (None, _) => format!("There is no program at {configured}."),
                };
                data.set_mpv_found(path.is_some());
                data.set_mpv_status(status.into());
            },
        );
    }

    fn add_language(&self, option: i32) {
        let Some((code, _)) = usize::try_from(option).ok().and_then(|i| LANGUAGES.get(i)) else {
            return;
        };
        {
            let mut settings = self.settings.borrow_mut();
            if settings.subtitle_languages.iter().any(|c| c == code) {
                return;
            }
            settings.subtitle_languages.push((*code).to_owned());
        }
        self.save_settings();
        self.fill_player();
    }

    fn remove_language(&self, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        {
            let mut settings = self.settings.borrow_mut();
            if index < settings.subtitle_languages.len() {
                settings.subtitle_languages.remove(index);
            }
        }
        self.save_settings();
        self.fill_player();
    }

    fn fill_files(&self) {
        let paths = self.paths.clone();
        spawn(
            move || {
                let size = paths.cache_size();
                (paths, size)
            },
            |s, (paths, size)| {
                let Some(app) = s.app.upgrade() else {
                    return;
                };
                let files = vec![
                    fact(
                        "Settings",
                        paths.config_dir().join("settings.json").as_str(),
                    ),
                    fact(
                        "Cache",
                        &format!("{} · {}", paths.cache_dir(), megabytes(size)),
                    ),
                ];
                app.global::<SettingsData>()
                    .set_files(ModelRc::new(VecModel::from(files)));
            },
        );
    }

    fn clear_cache(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.global::<SettingsData>().set_clearing(true);
        let paths = self.paths.clone();
        spawn(
            move || paths.clear_cache(),
            |s, result| {
                if let Some(app) = s.app.upgrade() {
                    app.global::<SettingsData>().set_clearing(false);
                }
                match result {
                    Ok(()) => s.toast("Cache cleared"),
                    Err(e) => eprintln!("clear cache: {e}"),
                }
                s.fill_files();
            },
        );
    }
}

fn open(url: &str) {
    if let Err(e) = open::that_detached(url) {
        eprintln!("open the browser: {e}");
    }
}

fn fact(label: &str, value: &str) -> Fact {
    Fact {
        label: label.into(),
        value: value.into(),
    }
}

/// For example `214 titles` or `1 title`.
fn plural(count: usize, what: &str) -> String {
    if count == 1 {
        format!("1 {what}")
    } else {
        format!("{count} {what}s")
    }
}

/// For example `just now`, `2 min ago` or `3 h ago`.
fn ago(when: Timestamp) -> String {
    let minutes = (Timestamp::now() - when).get_seconds() / 60;
    match minutes {
        m if m < 1 => "just now".to_owned(),
        m if m < 60 => format!("{m} min ago"),
        m => format!("{} h ago", m / 60),
    }
}

/// For example `184 MB`.
fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(MEGABYTE))
}

/// The version with the commit it was built from, for example `anchor
/// 0.1.0 · 7542fa1 of 2026-10-04`.
fn build() -> String {
    let version = format!("anchor {}", env!("CARGO_PKG_VERSION"));
    match env!("ANCHOR_COMMIT").split_once(' ') {
        Some((hash, date)) => format!("{version} · {hash} of {date}"),
        None => version,
    }
}

/// An addon as Settings lists it: a tile, its name, version, host and
/// what it does.
fn addon_item(addon: &Addon) -> AddonItem {
    let manifest = &addon.manifest;
    let resources: Vec<&str> = manifest
        .resources
        .iter()
        .map(|r| match r {
            anchor::addon::Resource::Short(name) | anchor::addon::Resource::Full { name, .. } => {
                name.as_str()
            }
        })
        .collect();
    let mut does = Vec::new();
    if manifest.catalogs.iter().any(|c| c.browsable()) {
        does.push("Catalogs");
    }
    if resources.contains(&"meta") {
        does.push("Metadata");
    }
    if manifest.catalogs.iter().any(|c| c.searchable()) {
        does.push("Search");
    }
    if resources.contains(&"stream") {
        does.push("Streams");
    }
    if resources.contains(&"subtitles") {
        does.push("Subtitles");
    }
    AddonItem {
        short: short_name(&manifest.name).into(),
        tint: text::tint(&manifest.id),
        name: manifest.name.as_str().into(),
        version: manifest.version.as_str().into(),
        host: host(&addon.transport_url).into(),
        does: ModelRc::new(VecModel::from(
            does.into_iter().map(SharedString::from).collect::<Vec<_>>(),
        )),
    }
}

/// Two letters for an addon's tile: the first and last capitals of its
/// name (`AIOStreams` is `AS`), else its first two letters.
fn short_name(name: &str) -> String {
    let capitals: Vec<char> = name.chars().filter(char::is_ascii_uppercase).collect();
    match capitals.as_slice() {
        [first, .., last] => [*first, *last].iter().collect(),
        _ => name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_uppercase(),
    }
}

/// The host of an addon's URL, without the path: the path can hold the
/// addon's configuration.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_and_hosts() {
        assert_eq!(short_name("AIOStreams"), "AS");
        assert_eq!(short_name("AIOMetadata"), "AM");
        assert_eq!(short_name("OpenSubtitles v3"), "OS");
        assert_eq!(short_name("Cinemeta"), "CI");
        assert_eq!(
            host("https://aio.example.net/eyJzZWNyZXQiOjF9/manifest.json"),
            "aio.example.net"
        );
        assert_eq!(
            host("http://127.0.0.1:8099/meta/manifest.json"),
            "127.0.0.1:8099"
        );
    }

    #[test]
    fn counts_and_sizes() {
        assert_eq!(plural(1, "title"), "1 title");
        assert_eq!(plural(214, "title"), "214 titles");
        assert_eq!(megabytes(0), "0 MB");
        assert_eq!(megabytes(184 * MEGABYTE - 5), "184 MB");
        assert_eq!(ago(Timestamp::now()), "just now");
    }
}
