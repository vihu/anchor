//! The Stremio account: signing in and out, the key in the keychain, and
//! keeping the addons and library in step with the account.

use std::rc::Rc;

use anchor::addon::{Addon, Catalog, MetaPreview};
use anchor::api::{self, Session as ApiSession};
use anchor::library::LibraryItem;
use anchor::net::Client;
use anchor::sources::Sources;
use anchor::store::Account;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

use super::{Session, spawn};
use crate::art::Size;
use crate::ui::{Screen, Shell};

/// Where a new account is made.
pub(super) const REGISTER_URL: &str = "https://www.stremio.com/register";
/// A public catalog whose posters fill the sign-in screen's wall.
const WALL_CATALOG: &str = "https://v3-cinemeta.strem.io/manifest.json";
/// Posters on the wall.
const WALL_POSTERS: usize = 24;
/// Cached answers, by name.
const ADDONS_CACHE: &str = "addons";
const LIBRARY_CACHE: &str = "library";

impl Session {
    /// Opens the saved account from the cache and syncs it, or shows the
    /// sign-in screen.
    pub(super) fn open_saved(self: &Rc<Self>) {
        let account = match self.paths.load_account() {
            Ok(Some(account)) => account,
            Ok(None) => return self.show_login("", ""),
            Err(e) => {
                eprintln!("account: {e}");
                return self.show_login("", "");
            }
        };
        let key = match account.key() {
            Ok(key) => key,
            Err(e) => {
                eprintln!("keychain: {e}");
                let email = account.email.clone();
                return self.show_login(
                    &email,
                    "The system keychain has no key for this account. Sign in again.",
                );
            }
        };
        let addons: Vec<Addon> = self.paths.cached(ADDONS_CACHE).unwrap_or_default();
        let library: Vec<LibraryItem> = self.paths.cached(LIBRARY_CACHE).unwrap_or_default();
        let cached = !addons.is_empty();
        self.signed_in(account, key);
        self.apply(addons, library);
        self.set_note(if cached { "" } else { "Loading your addons…" });
        self.show(Screen::Home);
        self.sync();
    }

    /// The sign-in screen's button: signs in, then keeps the key and syncs.
    pub(super) fn login(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let email = app.get_login_email().trim().to_owned();
        let password = app.get_login_password().to_string();
        if email.is_empty() || password.is_empty() || app.get_login_busy() {
            return;
        }
        app.set_login_busy(true);
        app.set_login_error(SharedString::new());
        let api = self.api.clone();
        spawn(
            move || {
                let session = api.login(&email, &password).map_err(|e| login_error(&e))?;
                let account = Account {
                    id: session.user.id.clone(),
                    email: session.user.email.clone(),
                };
                account
                    .save_key(&session.key)
                    .map_err(|e| format!("The system keychain refused the key: {e}"))?;
                Ok::<(Account, ApiSession), String>((account, session))
            },
            |s, result| {
                let Some(app) = s.app.upgrade() else {
                    return;
                };
                app.set_login_busy(false);
                match result {
                    Ok((account, session)) => {
                        if let Err(e) = s.paths.save_account(&account) {
                            eprintln!("account: {e}");
                        }
                        app.set_login_password(SharedString::new());
                        s.signed_in(account, session.key);
                        s.apply(Vec::new(), Vec::new());
                        s.set_note("Loading your addons…");
                        s.show(Screen::Home);
                        s.sync();
                    }
                    Err(message) => {
                        app.set_login_error(message.into());
                        app.invoke_focus_screen();
                    }
                }
            },
        );
    }

    /// Signs out: ends the session at Stremio, forgets the key and what was
    /// cached, and shows the sign-in screen.
    pub(super) fn sign_out(self: &Rc<Self>) {
        let account = {
            let mut state = self.state.borrow_mut();
            state.epoch += 1;
            state.library.clear();
            state.account.take()
        };
        let email = account
            .as_ref()
            .map(|(a, _)| a.email.clone())
            .unwrap_or_default();
        if let Some((account, key)) = account {
            let api = self.api.clone();
            // Best effort: the key is forgotten here either way.
            std::thread::spawn(move || {
                let _ = api.logout(&key);
                if let Err(e) = account.forget_key() {
                    eprintln!("keychain: {e}");
                }
            });
        }
        if let Err(e) = self.paths.forget_account() {
            eprintln!("sign out: {e}");
        }
        self.apply(Vec::new(), Vec::new());
        self.show_login(&email, "");
    }

    /// Fetches the account's addons and library, then shows them.
    pub(super) fn sync(self: &Rc<Self>) {
        let Some((key, epoch)) = self.key() else {
            return;
        };
        let api = self.api.clone();
        spawn(
            move || {
                api.addons(&key)
                    .and_then(|addons| Ok((addons, api.library(&key)?)))
            },
            move |s, result| {
                if !s.current(epoch) {
                    return;
                }
                match result {
                    Ok((addons, library)) => {
                        s.cache(&addons, &library);
                        s.apply(addons, library);
                        s.state.borrow_mut().synced_at = Some(jiff::Timestamp::now());
                        s.set_meta("synced");
                        let empty = s.state.borrow().sections.is_empty();
                        s.set_note(if empty {
                            "None of your addons has a catalog. Add one on web.stremio.com (AIOMetadata or Cinemeta, say), then Sync now in Settings."
                        } else {
                            ""
                        });
                        s.fill_settings();
                    }
                    Err(e) if e.signed_out() => {
                        let email = s.email();
                        s.sign_out();
                        s.show_login(&email, "Your Stremio session has expired. Sign in again.");
                    }
                    Err(e) => {
                        eprintln!("sync: {e}");
                        s.set_meta("offline");
                        s.set_note(&format!("Could not reach Stremio: {e}"));
                    }
                }
            },
        );
    }

    /// Records `item` in the library and its cache, and writes it to the
    /// account in the background.
    pub(super) fn save_item(&self, item: LibraryItem) {
        {
            let mut state = self.state.borrow_mut();
            match state.library.iter_mut().find(|i| i.id == item.id) {
                Some(slot) => *slot = item.clone(),
                None => state.library.push(item.clone()),
            }
            if let Err(e) = self.paths.cache(LIBRARY_CACHE, &state.library) {
                eprintln!("cache: {e}");
            }
        }
        let Some((key, _)) = self.key() else {
            return;
        };
        let api = self.api.clone();
        std::thread::spawn(move || {
            if let Err(e) = api.put_library(&key, &[item]) {
                eprintln!("library: {e}");
            }
        });
    }

    /// The signed-in account's email.
    pub(super) fn email(&self) -> String {
        self.state
            .borrow()
            .account
            .as_ref()
            .map(|(a, _)| a.email.clone())
            .unwrap_or_default()
    }
}

// Private
impl Session {
    fn signed_in(&self, account: Account, key: String) {
        if let Some(app) = self.app.upgrade() {
            let shell = app.global::<Shell>();
            shell.set_avatar(initials(&account.email).into());
            shell.set_email(account.email.as_str().into());
            shell.set_meta("syncing…".into());
        }
        let mut state = self.state.borrow_mut();
        state.epoch += 1;
        state.account = Some((account, key));
    }

    /// Takes `addons` and `library` as the account's.
    fn apply(self: &Rc<Self>, addons: Vec<Addon>, library: Vec<LibraryItem>) {
        let sources = Sources::new(addons);
        {
            let mut state = self.state.borrow_mut();
            state.sections = sources.sections();
            state.sources = sources;
            state.library = library;
        }
        self.apply_sidebar();
        self.refresh_home();
    }

    fn cache(&self, addons: &[Addon], library: &[LibraryItem]) {
        for result in [
            self.paths.cache(ADDONS_CACHE, &addons),
            self.paths.cache(LIBRARY_CACHE, &library),
        ] {
            if let Err(e) = result {
                eprintln!("cache: {e}");
            }
        }
    }

    fn set_meta(&self, what: &str) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let count = self.state.borrow().sources.addons().len();
        let addons = if count == 1 { "addon" } else { "addons" };
        app.global::<Shell>()
            .set_meta(format!("{count} {addons} · {what}").into());
    }

    fn set_note(&self, note: &str) {
        if let Some(app) = self.app.upgrade() {
            app.set_home_note(note.into());
        }
    }

    fn show_login(self: &Rc<Self>, email: &str, error: &str) {
        if let Some(app) = self.app.upgrade() {
            app.set_login_email(email.into());
            app.set_login_password(SharedString::new());
            app.set_login_error(error.into());
            app.set_login_busy(false);
        }
        self.show(Screen::Login);
        self.load_wall();
    }

    /// Fills the sign-in screen's wall with a public catalog's posters.
    fn load_wall(self: &Rc<Self>) {
        if !self.state.borrow().wall.is_empty() {
            return;
        }
        let addons = self.addons.clone();
        spawn(
            move || {
                let manifest = Client::new().get(WALL_CATALOG).ok()?;
                let addon = Addon {
                    transport_url: WALL_CATALOG.to_owned(),
                    manifest,
                };
                let catalog: Catalog = addon
                    .manifest
                    .catalogs
                    .iter()
                    .find(|c| c.browsable())?
                    .clone();
                addons.catalog(&addon, &catalog, &[]).ok()
            },
            |s, metas: Option<Vec<MetaPreview>>| {
                let urls: Vec<String> = metas
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|m| m.poster)
                    .take(WALL_POSTERS)
                    .collect();
                let images = Rc::new(VecModel::from(vec![Image::default(); urls.len()]));
                {
                    let mut state = s.state.borrow_mut();
                    state.wall.clone_from(&urls);
                    state.wall_images = Rc::clone(&images);
                }
                if let Some(app) = s.app.upgrade() {
                    app.set_login_posters(ModelRc::from(images));
                }
                let mut art = s.art.borrow_mut();
                for url in &urls {
                    art.request(url, Size::Poster);
                }
            },
        );
    }
}

/// What to say when signing in fails.
fn login_error(e: &api::Error) -> String {
    match e {
        api::Error::Api { message, .. } if message.to_lowercase().contains("not found") => {
            "No Stremio account has this email address.".to_owned()
        }
        api::Error::Api { message, .. }
            if ["password", "passphrase"]
                .iter()
                .any(|w| message.to_lowercase().contains(w)) =>
        {
            "Wrong email or password.".to_owned()
        }
        api::Error::Api { message, .. } => message.clone(),
        api::Error::Net(e) => format!("Could not reach Stremio: {e}."),
        _ => e.to_string(),
    }
}

/// Two letters for the account chip, from the email address: the first
/// letters of the first two words of its name, else its first two letters.
fn initials(email: &str) -> String {
    let name = email.split('@').next().unwrap_or_default();
    let words: Vec<&str> = name
        .split(['.', '_', '-', '+'])
        .filter(|w| !w.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
        _ => name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect(),
    };
    letters.to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_from_the_email() {
        assert_eq!(initials("alex.rivera@example.com"), "AR");
        assert_eq!(initials("alex@example.com"), "AL");
        assert_eq!(initials("x@example.com"), "X");
        assert_eq!(initials("@example.com"), "");
    }

    #[test]
    fn login_errors_in_plain_words() {
        let api = |message: &str| api::Error::Api {
            code: 2,
            message: message.into(),
        };
        assert_eq!(
            login_error(&api("User not found")),
            "No Stremio account has this email address."
        );
        assert_eq!(
            login_error(&api("Wrong passphrase")),
            "Wrong email or password."
        );
        assert_eq!(login_error(&api("Something else")), "Something else");
    }
}
