//! Where anchor keeps things: the settings and the signed-in account in the
//! config directory, the account's auth key in the OS keychain, the stream
//! last played per video in the data directory, and answers cached between
//! runs in the cache directory.
//!
//! The auth key never reaches a file. Cached answers hold the addon
//! collection, whose addon URLs can carry an addon's configuration, so
//! every file is written readable by its owner only.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;

use camino::{Utf8Path, Utf8PathBuf};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// Result type for this module.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Keychain service the auth key is stored under.
const KEYCHAIN_SERVICE: &str = "anchor";
/// File in the config directory that holds the settings.
const SETTINGS_FILE: &str = "settings.json";
/// File in the config directory that names the signed-in account.
const ACCOUNT_FILE: &str = "account.json";
/// File in the data directory with the stream last played per video.
const LAST_STREAMS_FILE: &str = "last-streams.json";
/// Directory in the cache directory for posters and backdrops.
const ART_DIR: &str = "art";
/// Only the owner reads or writes anchor's files.
const PRIVATE: u32 = 0o600;

/// Where anchor keeps its settings, its data and its cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    config: Utf8PathBuf,
    data: Utf8PathBuf,
    cache: Utf8PathBuf,
}

/// The signed-in Stremio account. Its auth key lives in the OS keychain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// The account's id at Stremio.
    pub id: String,
    /// The email address it signs in with.
    pub email: String,
}

/// The stream last played for a video, to select it again: what the addon
/// called it, never its URL, which carries debrid keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastStream {
    /// The stream's `name`.
    pub name: String,
    /// The stream's description.
    pub description: String,
}

/// What can go wrong reading or writing anchor's files and keys.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The OS reports no home directory to put settings in.
    NoHome,
    /// Reading or writing a file failed.
    Io(Utf8PathBuf, io::Error),
    /// A file is not valid JSON.
    Json(Utf8PathBuf, serde_json::Error),
    /// The OS keychain refused to store or return the auth key.
    Keychain(keyring::Error),
}

// Public API
impl Paths {
    /// The platform's standard locations, for example `~/.config/anchor`,
    /// `~/.local/share/anchor` and `~/.cache/anchor` on Linux.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoHome`] when the OS reports no home directory.
    pub fn system() -> Result<Self> {
        let dirs =
            directories::ProjectDirs::from("dev", "MotleyCode", "anchor").ok_or(Error::NoHome)?;
        let utf8 = |p: &std::path::Path| {
            Utf8PathBuf::from_path_buf(p.to_owned()).map_err(|_| Error::NoHome)
        };
        Ok(Self {
            config: utf8(dirs.config_dir())?,
            data: utf8(dirs.data_dir())?,
            cache: utf8(dirs.cache_dir())?,
        })
    }

    /// Settings under `root/config`, data under `root/data` and the cache
    /// under `root/cache`.
    pub fn under(root: &Utf8Path) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
        }
    }

    /// Loads the settings; the defaults when none are saved.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the file exists but cannot be read, and
    /// [`Error::Json`] when it is not valid JSON.
    pub fn load_settings(&self) -> Result<Settings> {
        Ok(read_json(&self.config.join(SETTINGS_FILE))?.unwrap_or_default())
    }

    /// Saves the settings.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the config directory cannot be written.
    pub fn save_settings(&self, settings: &Settings) -> Result {
        write_json(&self.config.join(SETTINGS_FILE), settings)
    }

    /// The signed-in account, if any.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] or [`Error::Json`] when the file is there but
    /// cannot be read.
    pub fn load_account(&self) -> Result<Option<Account>> {
        read_json(&self.config.join(ACCOUNT_FILE))
    }

    /// Saves `account` as the signed-in one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the config directory cannot be written.
    pub fn save_account(&self, account: &Account) -> Result {
        write_json(&self.config.join(ACCOUNT_FILE), account)
    }

    /// Forgets the signed-in account and what was cached for it; the
    /// settings stay.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when a file cannot be removed.
    pub fn forget_account(&self) -> Result {
        remove_file(&self.config.join(ACCOUNT_FILE))?;
        remove_file(&self.data.join(LAST_STREAMS_FILE))?;
        self.clear_cache()
    }

    /// A cached answer saved as `name`; `None` when there is none or it no
    /// longer reads (a cache is never worth an error).
    pub fn cached<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        read_json(&self.cache.join(format!("{name}.json")))
            .ok()
            .flatten()
    }

    /// Caches `value` as `name`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the cache directory cannot be written.
    pub fn cache<T: Serialize>(&self, name: &str, value: &T) -> Result {
        write_json(&self.cache.join(format!("{name}.json")), value)
    }

    /// The stream last played for each video id.
    pub fn last_streams(&self) -> HashMap<String, LastStream> {
        read_json(&self.data.join(LAST_STREAMS_FILE))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Saves the stream last played for each video id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the data directory cannot be written.
    pub fn save_last_streams(&self, streams: &HashMap<String, LastStream>) -> Result {
        write_json(&self.data.join(LAST_STREAMS_FILE), streams)
    }

    /// Where posters and backdrops are cached.
    pub fn art_dir(&self) -> Utf8PathBuf {
        self.cache.join(ART_DIR)
    }

    /// The settings directory.
    pub fn config_dir(&self) -> &Utf8Path {
        &self.config
    }

    /// The cache directory.
    pub fn cache_dir(&self) -> &Utf8Path {
        &self.cache
    }

    /// Bytes the cache takes on disk.
    pub fn cache_size(&self) -> u64 {
        walkdir::WalkDir::new(&self.cache)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .filter(fs::Metadata::is_file)
            .map(|meta| meta.len())
            .sum()
    }

    /// Empties the cache: answers, posters and backdrops.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the cache cannot be removed.
    pub fn clear_cache(&self) -> Result {
        match fs::remove_dir_all(&self.cache) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(self.cache.clone(), e)),
        }
    }
}

impl Account {
    /// Stores the account's auth key in the OS keychain.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Keychain`] when the keychain is locked or missing.
    pub fn save_key(&self, key: &str) -> Result {
        self.keychain_entry()?
            .set_password(key)
            .map_err(Error::Keychain)
    }

    /// The account's auth key, from the OS keychain.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Keychain`] when no key is stored or the keychain is
    /// unavailable.
    pub fn key(&self) -> Result<String> {
        self.keychain_entry()?
            .get_password()
            .map_err(Error::Keychain)
    }

    /// Deletes the account's auth key from the OS keychain; a missing one is
    /// not an error.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Keychain`] when the keychain is locked or missing.
    pub fn forget_key(&self) -> Result {
        match self.keychain_entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Keychain(e)),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::NoHome => None,
            Error::Io(_, e) => Some(e),
            Error::Json(_, e) => Some(e),
            Error::Keychain(e) => Some(e),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::NoHome => write!(f, "the system reports no home directory"),
            Error::Io(path, e) => write!(f, "could not read or write {path}: {e}"),
            Error::Json(path, e) => write!(f, "{path} is not valid JSON: {e}"),
            Error::Keychain(e) => write!(f, "the system keychain: {e}"),
        }
    }
}

// Private API
impl Account {
    fn keychain_entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &self.id).map_err(Error::Keychain)
    }
}

/// Reads JSON from `path`; `None` when there is no file.
fn read_json<T: DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| Error::Json(path.to_owned(), e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(path.to_owned(), e)),
    }
}

/// Writes `value` as JSON through a temporary file renamed over `path`, so
/// a crash never leaves half a file, readable by the owner only.
fn write_json<T: Serialize>(path: &Utf8Path, value: &T) -> Result {
    let io_err = |e| Error::Io(path.to_owned(), e);
    let json = serde_json::to_vec(value).expect("anchor's types always serialize");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let tmp = path.with_extension("tmp");
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(PRIVATE)
        .open(&tmp)
        .and_then(|mut file| file.write_all(&json))
        .map_err(io_err)?;
    fs::rename(&tmp, path).map_err(io_err)
}

fn remove_file(path: &Utf8Path) -> Result {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Io(path.to_owned(), e)),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::settings::Sidebar;

    fn temp_paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap().to_owned();
        (dir, Paths::under(&root))
    }

    fn account() -> Account {
        Account {
            id: "5f2a".into(),
            email: "alex@example.com".into(),
        }
    }

    #[test]
    fn settings_default_when_missing_and_round_trip() {
        let (_dir, paths) = temp_paths();
        assert_eq!(paths.load_settings().unwrap(), Settings::default());
        let settings = Settings {
            sidebar: Sidebar::Collapsed,
            ..Settings::default()
        };
        paths.save_settings(&settings).unwrap();
        assert_eq!(paths.load_settings().unwrap(), settings);
    }

    #[test]
    fn a_damaged_settings_file_is_an_error() {
        let (_dir, paths) = temp_paths();
        fs::create_dir_all(&paths.config).unwrap();
        fs::write(paths.config.join(SETTINGS_FILE), "{not json").unwrap();
        assert!(matches!(paths.load_settings(), Err(Error::Json(..))));
    }

    #[test]
    fn files_are_private() {
        let (_dir, paths) = temp_paths();
        paths.cache("collection", &["addon"]).unwrap();
        let mode = fs::metadata(paths.cache.join("collection.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, PRIVATE);
    }

    #[test]
    fn forgetting_the_account_keeps_the_settings() {
        let (_dir, paths) = temp_paths();
        paths.save_settings(&Settings::default()).unwrap();
        paths.save_account(&account()).unwrap();
        paths.cache("library", &[1, 2, 3]).unwrap();
        let mut last = HashMap::new();
        last.insert(
            "tt1:1:2".to_owned(),
            LastStream {
                name: "[TB+] 2160p".into(),
                description: "Small.Thieves.S01E02".into(),
            },
        );
        paths.save_last_streams(&last).unwrap();
        assert_eq!(paths.load_account().unwrap(), Some(account()));
        assert_eq!(paths.cached::<Vec<i32>>("library"), Some(vec![1, 2, 3]));
        assert_eq!(paths.last_streams(), last);
        assert!(paths.cache_size() > 0);

        paths.forget_account().unwrap();
        assert_eq!(paths.load_account().unwrap(), None);
        assert_eq!(paths.cached::<Vec<i32>>("library"), None);
        assert!(paths.last_streams().is_empty());
        assert_eq!(paths.cache_size(), 0);
        assert!(paths.config.join(SETTINGS_FILE).exists());
        // Twice is fine: nothing left to remove.
        paths.forget_account().unwrap();
    }

    #[test]
    fn a_damaged_cache_is_a_miss() {
        let (_dir, paths) = temp_paths();
        fs::create_dir_all(&paths.cache).unwrap();
        fs::write(paths.cache.join("library.json"), "[1,").unwrap();
        assert_eq!(paths.cached::<Vec<i32>>("library"), None);
    }
}
