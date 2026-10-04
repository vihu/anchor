//! What the window shows and why: the account, its addons and library, the
//! screens, and mpv.
//!
//! The session lives on the UI thread. Network and keychain work runs on
//! worker threads; results come back through `slint::invoke_from_event_loop`
//! (itele's pattern). Stream and addon URLs carry keys, so none is logged
//! or shown.

mod account;
mod shell;

use std::cell::RefCell;
use std::rc::Rc;
use std::thread;

use anchor::addon::Addons;
use anchor::api::Api;
use anchor::library::LibraryItem;
use anchor::settings::Settings;
use anchor::sources::{Section, Sources};
use anchor::store::{Account, Paths};
use slint::{ComponentHandle, Image, Model, VecModel};

use crate::art::{Art, Picture, Size};
use crate::ui::{AppWindow, Screen, Shell};

thread_local! {
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

/// The window's controller. Lives on the UI thread.
pub struct Session {
    app: slint::Weak<AppWindow>,
    paths: Paths,
    settings: RefCell<Settings>,
    api: Api,
    addons: Addons,
    state: RefCell<State>,
    art: RefCell<Art>,
}

/// What the session knows.
#[derive(Default)]
struct State {
    /// The signed-in account and its key; `None` on the sign-in screen.
    account: Option<(Account, String)>,
    /// Bumped on sign-out: answers for an earlier account are dropped.
    epoch: u64,
    sources: Sources,
    sections: Vec<Section>,
    library: Vec<LibraryItem>,
    /// The sign-in screen's posters, by URL, in the wall's order.
    wall: Vec<String>,
    wall_images: Rc<VecModel<Image>>,
}

/// Wires `app` to a new session and opens the saved account, or the
/// sign-in screen.
pub fn start(app: &AppWindow, paths: Paths, api: Api, addons: Addons) {
    let art = Art::start(paths.art_dir(), |url, size, picture| {
        on_ui_thread(move |s| s.art_ready(url, size, picture));
    });
    let settings = paths
        .load_settings()
        .inspect_err(|e| eprintln!("settings: {e}"))
        .unwrap_or_default();
    let session = Rc::new(Session {
        app: app.as_weak(),
        paths,
        settings: RefCell::new(settings),
        api,
        addons,
        state: RefCell::default(),
        art: RefCell::new(art),
    });
    SESSION.with(|s| *s.borrow_mut() = Some(Rc::clone(&session)));

    app.on_login(|| with_session(|s| s.login()));
    app.on_create_account(|| {
        if let Err(e) = open::that_detached(account::REGISTER_URL) {
            eprintln!("open the browser: {e}");
        }
    });
    app.on_sign_out(|| with_session(|s| s.sign_out()));
    app.on_navigate(|i| with_session(|s| s.navigate(i)));
    app.on_back(|| with_session(|s| s.back()));
    let shell = app.global::<Shell>();
    shell.on_toggle_sidebar(|| with_session(|s| s.toggle_sidebar()));
    shell.on_fold(|i| with_session(|s| s.fold(i)));
    shell.on_open_catalog(|s, c| with_session(|session| session.open_catalog(s, c)));

    session.apply_sidebar();
    session.open_saved();
}

/// Saves what is left to save before the window closes.
pub fn finish() {}

/// Runs `f` with the session, if the window still has one.
fn with_session(f: impl FnOnce(&Rc<Session>)) {
    // Clone out first: `f` may re-enter through a callback.
    if let Some(session) = SESSION.with(|s| s.borrow().clone()) {
        f(&session);
    }
}

/// Runs `f` with the session on the UI thread, from any thread.
fn on_ui_thread(f: impl FnOnce(&Rc<Session>) + Send + 'static) {
    // Err only once the event loop has quit; nothing is left to update.
    let _ = slint::invoke_from_event_loop(move || with_session(f));
}

/// Runs `work` on a worker thread, then `done` with its result on the UI
/// thread.
fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&Rc<Session>, T) + Send + 'static,
) {
    thread::spawn(move || {
        let result = work();
        on_ui_thread(move |s| done(s, result));
    });
}

// Helpers
impl Session {
    fn show(&self, screen: Screen) {
        if let Some(app) = self.app.upgrade() {
            app.set_screen(screen);
            app.invoke_focus_screen();
        }
    }

    /// The account's key and the epoch it belongs to, when signed in.
    fn key(&self) -> Option<(String, u64)> {
        let state = self.state.borrow();
        state
            .account
            .as_ref()
            .map(|(_, key)| (key.clone(), state.epoch))
    }

    /// Whether an answer started in `epoch` still belongs to this account.
    fn current(&self, epoch: u64) -> bool {
        self.state.borrow().epoch == epoch
    }

    fn save_settings(&self) {
        if let Err(e) = self.paths.save_settings(&self.settings.borrow()) {
            eprintln!("settings: {e}");
        }
    }

    /// A picture arrived: every screen showing it takes it.
    fn art_ready(&self, url: String, size: Size, picture: Option<Picture>) {
        self.art.borrow_mut().finish(&url, size);
        let Some(image) = picture.and_then(Picture::image) else {
            return;
        };
        let state = self.state.borrow();
        if size == Size::Poster {
            for (i, wall_url) in state.wall.iter().enumerate() {
                if *wall_url == url && i < state.wall_images.row_count() {
                    state.wall_images.set_row_data(i, image.clone());
                }
            }
        }
    }
}
