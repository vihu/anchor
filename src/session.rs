//! What the window shows and why: the account, its addons and library, the
//! screens, and mpv.
//!
//! The session lives on the UI thread. Network and keychain work runs on
//! worker threads; results come back through `slint::invoke_from_event_loop`
//! (itele's pattern). Stream and addon URLs carry keys, so none is logged
//! or shown.

mod account;
mod catalog;
mod home;
mod playing;
mod search;
mod settings;
mod shell;
mod streams;
mod title;
mod writer;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::thread;

use anchor::addon::{Addons, MetaItem};
use anchor::api::Api;
use anchor::library::LibraryItem;
use anchor::settings::Settings;
use anchor::sources::{Section, Sources};
use anchor::store::{Account, Paths};
use slint::{ComponentHandle, Image, Model, Timer, VecModel};

use crate::art::{Art, Picture, Size};
use crate::ui::{AppWindow, NowPlaying, Screen, SearchData, Shell};

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
    search_timer: Timer,
    toast_timer: Timer,
    writer: RefCell<writer::Writer>,
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
    home: home::Home,
    browse: catalog::Browse,
    search: search::Search,
    page: Option<title::Page>,
    panel: streams::Panel,
    playing: Option<playing::Playing>,
    /// Source of playback tokens.
    tokens: u64,
    /// When the account was last synced.
    synced_at: Option<jiff::Timestamp>,
    /// Titles' metadata fetched so far, and those on their way.
    metas: home::Metas,
    fetching: HashSet<String>,
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
    let writer = writer::Writer::start(api.clone());
    let session = Rc::new(Session {
        app: app.as_weak(),
        paths,
        settings: RefCell::new(settings),
        api,
        addons,
        state: RefCell::default(),
        art: RefCell::new(art),
        search_timer: Timer::default(),
        toast_timer: Timer::default(),
        writer: RefCell::new(writer),
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
    app.on_step_catalog(|step| with_session(|s| s.step_catalog(step)));
    app.on_back(|| with_session(|s| s.back()));
    let shell = app.global::<Shell>();
    shell.on_toggle_sidebar(|| with_session(|s| s.toggle_sidebar()));
    shell.on_fold(|i| with_session(|s| s.fold(i)));
    shell.on_open_catalog(|s, c| with_session(|session| session.open_catalog(s, c)));
    app.on_home_rows_visible(|first, count| with_session(|s| s.home_rows_visible(first, count)));
    app.on_home_see_all(|row| with_session(|s| s.see_all(row)));
    app.on_catalog_genre_selected(|g| with_session(|s| s.catalog_genre_selected(g)));
    app.on_catalog_items_visible(|first, count| {
        with_session(|s| s.catalog_items_visible(first, count));
    });
    app.on_home_play_hero(|| with_session(|s| s.play_hero()));
    app.on_home_open_hero(|| with_session(|s| s.open_with(Session::home_hero_preview)));
    app.on_home_open_card(|i| with_session(|s| s.open_with(|s| s.home_card_preview(i))));
    app.on_home_open_title(|r, i| with_session(|s| s.open_with(|s| s.home_row_preview(r, i))));
    app.on_catalog_open(|i| with_session(|s| s.open_with(|s| s.catalog_preview(i))));
    app.on_title_season_selected(|i| with_session(|s| s.title_season_selected(i)));
    app.on_title_toggle_watched(|| with_session(|s| s.title_toggle_watched()));
    app.on_title_play(|| with_session(|s| s.title_play(streams::Start::Resume)));
    app.on_title_restart(|| with_session(|s| s.title_play(streams::Start::Beginning)));
    app.on_play_stream(|i| with_session(|s| s.play_stream(i)));
    app.on_close_streams(|| with_session(|s| s.close_streams()));
    let now = app.global::<NowPlaying>();
    now.on_open(|| with_session(|s| s.open_playing()));
    now.on_stop(|| with_session(|s| s.stop_playing()));
    let search = app.global::<SearchData>();
    search.on_picked(|i| with_session(|s| s.open_with(|s| s.search_preview(i))));
    search.on_edited(|_| with_session(|s| s.search_edited()));
    search.on_move(|delta| with_session(|s| s.search_move(delta)));
    search.on_all(|| with_session(|s| s.search_all()));

    Session::wire_settings(app);
    session.apply_sidebar();
    session.open_saved();
}

/// Saves what is left to save before the window closes: the position of
/// the title playing.
pub fn finish() {
    with_session(|s| {
        s.finish_playing();
        s.writer.borrow_mut().finish();
    });
}

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
        if size == Size::Poster {
            let state = self.state.borrow();
            for (i, wall_url) in state.wall.iter().enumerate() {
                if *wall_url == url && i < state.wall_images.row_count() {
                    state.wall_images.set_row_data(i, image.clone());
                }
            }
        }
        self.home_art_ready(&url, size, &image);
        if size == Size::Still {
            self.playing_art_ready(&url, &image);
        }
        self.page_art_ready(&url, size, &image);
        if size == Size::Poster {
            self.catalog_art_ready(&url, &image);
            self.search_art_ready(&url, &image);
        }
    }

    /// Fetches the metadata of title `id` of type `kind` from the first addon
    /// that answers it, unless it is known or on its way.
    fn fetch_meta(self: &Rc<Self>, kind: &str, id: &str) {
        let addons: Vec<_> = {
            let mut state = self.state.borrow_mut();
            if state.metas.contains_key(id) || !state.fetching.insert(id.to_owned()) {
                return;
            }
            state
                .sources
                .serving("meta", kind, id)
                .into_iter()
                .cloned()
                .collect()
        };
        let client = self.addons.clone();
        let (kind, id) = (kind.to_owned(), id.to_owned());
        let asked = id.clone();
        spawn(
            move || {
                let mut last = None;
                for addon in &addons {
                    match client.meta(addon, &kind, &id) {
                        Ok(meta) => return Ok(meta),
                        Err(e) => last = Some(e),
                    }
                }
                Err(last.map_or_else(|| "no addon has this title".to_owned(), |e| e.to_string()))
            },
            move |s, result: Result<MetaItem, String>| {
                s.state.borrow_mut().fetching.remove(&asked);
                match result {
                    Ok(meta) => s.meta_ready(meta),
                    Err(e) => eprintln!("metadata: {e}"),
                }
            },
        );
    }

    /// Opens the page of the title `pick` names, if it names one.
    fn open_with(self: &Rc<Self>, pick: impl FnOnce(&Self) -> Option<anchor::addon::MetaPreview>) {
        if let Some(preview) = pick(self) {
            self.open_title(preview);
        }
    }

    /// A title's metadata arrived: kept, and handed to the screens.
    fn meta_ready(self: &Rc<Self>, meta: MetaItem) {
        self.state
            .borrow_mut()
            .metas
            .insert(meta.preview.id.clone(), meta.clone());
        self.home_meta_ready(&meta);
        self.page_meta_ready(&meta);
    }
}
