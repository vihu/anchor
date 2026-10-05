//! Home: the title to resume, Continue watching from the library, and a row
//! per catalog of the account's addons, each loaded on its own.
//!
//! Posters are kept only for the rows on screen and one either side, so a
//! dozen catalogs do not hold hundreds of decoded pictures.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use anchor::addon::{MetaItem, MetaPreview};
use anchor::library::{self, LibraryItem};
use anchor::sources::CatalogRef;
use slint::{Image, Model, ModelRc, SharedString, VecModel};

use super::{Session, spawn};
use crate::art::Size;
use crate::text;
use crate::ui::{CardItem, HeroItem, PosterItem, RowItem};

/// Titles of Continue watching whose pictures and episodes are fetched.
const CARDS_WITH_META: usize = 12;

/// What Home shows.
#[derive(Default)]
pub(super) struct Home {
    /// The catalogs of the rows, to tell when they change.
    catalogs: Vec<(String, String, String)>,
    rows: Vec<Row>,
    row_model: Rc<VecModel<RowItem>>,
    /// Rows allowed to hold posters.
    window: Range<usize>,
    /// The rows last reported on screen, as first and count.
    visible: (usize, usize),
    cards: Vec<Card>,
    card_model: Rc<VecModel<CardItem>>,
    /// The hero's backdrop.
    hero_backdrop: Option<String>,
    /// Bumped when the rows are built again: older answers are dropped.
    generation: u64,
}

/// One catalog row.
struct Row {
    at: CatalogRef,
    metas: Vec<MetaPreview>,
    items: Rc<VecModel<PosterItem>>,
}

/// One title in Continue watching.
struct Card {
    item: LibraryItem,
    image: Option<String>,
}

impl Session {
    /// Builds Home again from the library and the sources: Continue watching
    /// and the hero always, the rows when the catalogs changed.
    pub(super) fn refresh_home(self: &Rc<Self>) {
        self.refresh_cards();
        let catalogs = self.home_catalogs();
        if catalogs != self.state.borrow().home.catalogs {
            self.load_rows(catalogs);
        } else {
            self.mark_rows();
        }
        self.show_hero();
    }

    /// Catalog rows `first` to `first + count` are on screen: their posters,
    /// and one row either side, are kept; the rest are let go.
    pub(super) fn home_rows_visible(&self, first: i32, count: i32) {
        let urls: Vec<String> = {
            let mut state = self.state.borrow_mut();
            let home = &mut state.home;
            let len = home.rows.len();
            let first = usize::try_from(first).unwrap_or(0);
            let count = usize::try_from(count).unwrap_or(0);
            home.visible = (first, count);
            let window = first.saturating_sub(1).min(len)..(first + count + 1).min(len);
            for (r, row) in home.rows.iter().enumerate() {
                if !window.contains(&r) && home.window.contains(&r) {
                    for i in 0..row.items.row_count() {
                        let mut item = row.items.row_data(i).expect("in range");
                        item.poster = Image::default();
                        item.has_poster = false;
                        row.items.set_row_data(i, item);
                    }
                }
            }
            let urls = home.rows[window.clone()]
                .iter()
                .flat_map(|row| row.metas.iter().filter_map(|m| m.poster.clone()))
                .collect();
            home.window = window;
            urls
        };
        let mut art = self.art.borrow_mut();
        for url in &urls {
            art.request(url, Size::Poster);
        }
    }

    /// A picture for Home arrived.
    pub(super) fn home_art_ready(&self, url: &str, size: Size, image: &Image) {
        let state = self.state.borrow();
        let home = &state.home;
        match size {
            Size::Poster => {
                for row in &home.rows[home.window.clone()] {
                    for (i, meta) in row.metas.iter().enumerate() {
                        if meta.poster.as_deref() == Some(url) {
                            let mut item = row.items.row_data(i).expect("in range");
                            item.poster = image.clone();
                            item.has_poster = true;
                            row.items.set_row_data(i, item);
                        }
                    }
                }
            }
            Size::Still => {
                for (i, card) in home.cards.iter().enumerate() {
                    if card.image.as_deref() == Some(url) {
                        let mut item = home.card_model.row_data(i).expect("in range");
                        item.image = image.clone();
                        item.has_image = true;
                        home.card_model.set_row_data(i, item);
                    }
                }
            }
            Size::Backdrop => {
                if home.hero_backdrop.as_deref() == Some(url)
                    && let Some(app) = self.app.upgrade()
                {
                    let mut hero = app.get_home_hero();
                    hero.backdrop = image.clone();
                    hero.has_backdrop = true;
                    app.set_home_hero(hero);
                }
            }
            // Home shows no cast.
            Size::Face => {}
        }
    }

    /// The title of Continue watching's card `i`, as a catalog would list it.
    pub(super) fn home_card_preview(&self, i: i32) -> Option<MetaPreview> {
        let state = self.state.borrow();
        let item = &state.home.cards.get(usize::try_from(i).ok()?)?.item;
        Some(MetaPreview {
            id: item.id.clone(),
            kind: item.kind.clone(),
            name: item.name.clone(),
            poster: item.poster.clone(),
            ..MetaPreview::default()
        })
    }

    /// Title `i` of catalog row `row`.
    pub(super) fn home_row_preview(&self, row: i32, i: i32) -> Option<MetaPreview> {
        let state = self.state.borrow();
        let row = state.home.rows.get(usize::try_from(row).ok()?)?;
        row.metas.get(usize::try_from(i).ok()?).cloned()
    }

    /// The hero's title: the first card's, else the first row's first.
    pub(super) fn home_hero_preview(&self) -> Option<MetaPreview> {
        self.home_card_preview(0).or_else(|| {
            let state = self.state.borrow();
            state
                .home
                .rows
                .iter()
                .find_map(|r| r.metas.first().cloned())
        })
    }

    /// The hero's Resume: the title's page with the streams of the episode
    /// (or movie) the user is at.
    pub(super) fn play_hero(self: &Rc<Self>) {
        let Some(preview) = self.home_hero_preview() else {
            return;
        };
        let target = {
            let state = self.state.borrow();
            let item = state
                .home
                .cards
                .first()
                .map(|c| &c.item)
                .filter(|i| i.id == preview.id);
            item.and_then(|item| {
                let video_id = item.state.video_id.clone()?;
                let episode = text::episode(&video_id).filter(|_| item.kind != "movie");
                let code = episode.map(|(s, e)| text::code(s, e));
                let meta = state.metas.get(&item.id);
                let title = meta.and_then(|m| video_title(m, item)).unwrap_or_default();
                Some(super::streams::Target {
                    kind: item.kind.clone(),
                    meta_id: item.id.clone(),
                    video_id,
                    name: item.name.clone(),
                    label: super::streams::label(code.as_deref(), &title, ""),
                    code,
                    picture: state.home.cards.first().and_then(|c| c.image.clone()),
                })
            })
        };
        self.open_title(preview);
        if let Some(target) = target {
            self.open_streams(target, super::streams::Start::Resume, None);
        }
    }

    /// A title's metadata arrived (fetched for Home, a page, a search):
    /// Home's cards and hero take what they need from it.
    pub(super) fn home_meta_ready(&self, meta: &MetaItem) {
        let url = {
            let mut state = self.state.borrow_mut();
            let home = &mut state.home;
            let Some(i) = home.cards.iter().position(|c| c.item.id == meta.preview.id) else {
                return;
            };
            let image = meta
                .preview
                .background
                .clone()
                .or_else(|| meta.preview.poster.clone());
            home.cards[i].image.clone_from(&image);
            let mut card = home.card_model.row_data(i).expect("in range");
            if let Some(title) = video_title(meta, &home.cards[i].item) {
                card.detail = format!("{} · {title}", card.detail).into();
            }
            home.card_model.set_row_data(i, card);
            image
        };
        if let Some(url) = url {
            self.art.borrow_mut().request(&url, Size::Still);
        }
        self.show_hero();
    }
}

// Private
impl Session {
    /// The catalogs Home has rows for, by addon and catalog.
    fn home_catalogs(&self) -> Vec<(String, String, String)> {
        let state = self.state.borrow();
        state
            .sources
            .browsable()
            .into_iter()
            .filter_map(|at| state.sources.catalog(at))
            .map(|(addon, catalog)| {
                (
                    addon.transport_url.clone(),
                    catalog.kind.clone(),
                    catalog.id.clone(),
                )
            })
            .collect()
    }

    fn refresh_cards(self: &Rc<Self>) {
        let (cards, wanted) = {
            let state = self.state.borrow();
            let items: Vec<LibraryItem> = library::continue_watching(&state.library)
                .into_iter()
                .cloned()
                .collect();
            let wanted: Vec<(String, String)> = items
                .iter()
                .take(CARDS_WITH_META)
                .filter(|item| !state.metas.contains_key(&item.id))
                .map(|item| (item.kind.clone(), item.id.clone()))
                .collect();
            let cards: Vec<Card> = items
                .into_iter()
                .map(|item| Card { item, image: None })
                .collect();
            (cards, wanted)
        };
        let models: Vec<CardItem> = cards.iter().map(card_item).collect();
        {
            let mut state = self.state.borrow_mut();
            let home = &mut state.home;
            home.cards = cards;
            home.card_model = Rc::new(VecModel::from(models));
        }
        if let Some(app) = self.app.upgrade() {
            app.set_home_cards(ModelRc::from(Rc::clone(
                &self.state.borrow().home.card_model,
            )));
        }
        // Metadata already fetched shows at once; the rest is asked for.
        let known: Vec<MetaItem> = {
            let state = self.state.borrow();
            state
                .home
                .cards
                .iter()
                .take(CARDS_WITH_META)
                .filter_map(|c| state.metas.get(&c.item.id).cloned())
                .collect()
        };
        for meta in &known {
            self.home_meta_ready(meta);
        }
        for (kind, id) in wanted {
            self.fetch_meta(&kind, &id);
        }
    }

    /// Fills the hero: the first title of Continue watching, else the first
    /// title of the first row.
    fn show_hero(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (hero, backdrop) = {
            let state = self.state.borrow();
            let home = &state.home;
            if let Some(card) = home.cards.first() {
                let meta = state.metas.get(&card.item.id);
                resume_hero(&card.item, meta)
            } else if let Some((row, meta)) = home
                .rows
                .iter()
                .find_map(|row| row.metas.first().map(|m| (row, m)))
            {
                let title = row_title(&state, row.at);
                featured_hero(meta, title)
            } else {
                (HeroItem::default(), None)
            }
        };
        let same = backdrop.is_some() && backdrop == self.state.borrow().home.hero_backdrop;
        let mut hero = hero;
        if same {
            let current = app.get_home_hero();
            hero.backdrop = current.backdrop;
            hero.has_backdrop = current.has_backdrop;
        }
        app.set_home_hero(hero);
        self.state
            .borrow_mut()
            .home
            .hero_backdrop
            .clone_from(&backdrop);
        if let (Some(url), false) = (backdrop, same) {
            self.art.borrow_mut().request(&url, Size::Backdrop);
        }
    }

    /// Builds a row per catalog and loads each.
    fn load_rows(self: &Rc<Self>, catalogs: Vec<(String, String, String)>) {
        let (refs, items, generation) = {
            let mut state = self.state.borrow_mut();
            let refs = state.sources.browsable();
            let items: Vec<RowItem> = refs
                .iter()
                .map(|at| RowItem {
                    title: row_title(&state, *at).into(),
                    source: state
                        .sources
                        .catalog(*at)
                        .map(|(addon, _)| addon.manifest.name.as_str())
                        .unwrap_or_default()
                        .into(),
                    items: ModelRc::default(),
                    note: "Loading…".into(),
                })
                .collect();
            let home = &mut state.home;
            home.generation += 1;
            home.catalogs = catalogs;
            let (first, count) = home.visible;
            home.window =
                first.saturating_sub(1).min(refs.len())..(first + count + 1).min(refs.len());
            home.rows = refs
                .iter()
                .map(|at| Row {
                    at: *at,
                    metas: Vec::new(),
                    items: Rc::default(),
                })
                .collect();
            home.row_model = Rc::new(VecModel::from(items));
            (refs, Rc::clone(&home.row_model), home.generation)
        };
        if let Some(app) = self.app.upgrade() {
            app.set_home_rows(ModelRc::from(items));
        }
        for (r, at) in refs.into_iter().enumerate() {
            let Some((addon, catalog)) = self
                .state
                .borrow()
                .sources
                .catalog(at)
                .map(|(a, c)| (a.clone(), c.clone()))
            else {
                continue;
            };
            let addons = self.addons.clone();
            spawn(
                move || addons.catalog(&addon, &catalog, &[]),
                move |s, result| s.row_loaded(generation, r, result),
            );
        }
    }

    fn row_loaded(
        self: &Rc<Self>,
        generation: u64,
        r: usize,
        result: Result<Vec<MetaPreview>, anchor::net::Error>,
    ) {
        let (note, metas) = match result {
            Ok(metas) if metas.is_empty() => ("Nothing here.".to_owned(), metas),
            Ok(metas) => (String::new(), metas),
            Err(e) => (format!("Could not load this catalog: {e}."), Vec::new()),
        };
        let in_window = {
            let mut state = self.state.borrow_mut();
            if state.home.generation != generation {
                return;
            }
            let posters: Vec<PosterItem> = metas
                .iter()
                .map(|m| poster_item(m, &state.library))
                .collect();
            let home = &mut state.home;
            let Some(row) = home.rows.get_mut(r) else {
                return;
            };
            row.metas = metas;
            row.items = Rc::new(VecModel::from(posters));
            let mut item = home.row_model.row_data(r).expect("a row per catalog");
            item.items = ModelRc::from(Rc::clone(&row.items));
            item.note = SharedString::from(note);
            home.row_model.set_row_data(r, item);
            home.window.contains(&r)
        };
        if in_window {
            let urls: Vec<String> = self.state.borrow().home.rows[r]
                .metas
                .iter()
                .filter_map(|m| m.poster.clone())
                .collect();
            let mut art = self.art.borrow_mut();
            for url in &urls {
                art.request(url, Size::Poster);
            }
        }
        if r == 0 {
            self.show_hero();
        }
    }

    /// Marks watched titles and progress on the rows again, after the
    /// library changed.
    fn mark_rows(&self) {
        let state = self.state.borrow();
        for row in &state.home.rows {
            for (i, meta) in row.metas.iter().enumerate() {
                let fresh = poster_item(meta, &state.library);
                let mut item = row.items.row_data(i).expect("in range");
                item.progress = fresh.progress;
                item.watched = fresh.watched;
                row.items.set_row_data(i, item);
            }
        }
    }
}

/// A poster for `meta`, with its progress and watched mark from `library`.
pub(super) fn poster_item(meta: &MetaPreview, library: &[LibraryItem]) -> PosterItem {
    let item = library.iter().find(|i| i.id == meta.id);
    PosterItem {
        title: meta.name.as_str().into(),
        detail: text::year(meta.release_info.as_deref()).into(),
        rating: meta.rating().unwrap_or_default().into(),
        tint: text::tint(&meta.id),
        poster: Image::default(),
        has_poster: false,
        progress: item
            .filter(|i| i.in_continue_watching())
            .map_or(0.0, |i| i.progress() as f32),
        watched: item.is_some_and(LibraryItem::watched),
    }
}

fn card_item(card: &Card) -> CardItem {
    let item = &card.item;
    let video = item.state.video_id.as_deref().unwrap_or_default();
    let episode = text::episode(video).filter(|_| item.kind != "movie");
    let up_next = item.state.time_offset <= 1;
    let detail = match episode {
        Some((season, number)) => text::code(season, number),
        None if item.kind == "movie" => "Movie".to_owned(),
        None => String::new(),
    };
    CardItem {
        title: item.name.as_str().into(),
        detail: detail.into(),
        left: if up_next {
            SharedString::new()
        } else {
            text::left(item.state.time_offset, item.state.duration).into()
        },
        tag: if up_next {
            "Up next".into()
        } else {
            SharedString::new()
        },
        tint: text::tint(&item.id),
        image: Image::default(),
        has_image: false,
        progress: if up_next { 0.0 } else { item.progress() as f32 },
    }
}

/// The hero for resuming `item`, and its backdrop's URL.
fn resume_hero(item: &LibraryItem, meta: Option<&MetaItem>) -> (HeroItem, Option<String>) {
    let video = item.state.video_id.as_deref().unwrap_or_default();
    let episode = text::episode(video).filter(|_| item.kind != "movie");
    let up_next = item.state.time_offset <= 1;
    let code = episode.map(|(s, e)| text::code(s, e));
    let mut parts: Vec<String> = code.iter().cloned().collect();
    if let Some(title) = meta.and_then(|m| video_title(m, item)) {
        parts.push(title);
    }
    let verb = if up_next { "Play" } else { "Resume" };
    let hero = HeroItem {
        shown: true,
        eyebrow: "Continue watching".into(),
        title: item.name.as_str().into(),
        meta: parts.join(" · ").into(),
        rating: meta
            .and_then(|m| m.preview.rating())
            .unwrap_or_default()
            .into(),
        plot: meta
            .and_then(|m| episode_plot(m, video).or(m.preview.description.as_deref()))
            .unwrap_or_default()
            .into(),
        left: if up_next {
            SharedString::new()
        } else {
            text::left(item.state.time_offset, item.state.duration).into()
        },
        progress: if up_next { 0.0 } else { item.progress() as f32 },
        play_label: code
            .map_or_else(|| verb.to_owned(), |c| format!("{verb} {c}"))
            .into(),
        tint: text::tint(&item.id),
        backdrop: Image::default(),
        has_backdrop: false,
    };
    let backdrop = meta.and_then(|m| m.preview.background.clone());
    (hero, backdrop)
}

/// The hero featuring the first title of a row.
fn featured_hero(meta: &MetaPreview, row_title: String) -> (HeroItem, Option<String>) {
    let mut parts = vec![text::year(meta.release_info.as_deref())];
    if let Some(runtime) = &meta.runtime {
        parts.push(text::runtime(runtime));
    }
    let genres = meta.genre_names();
    if !genres.is_empty() {
        parts.push(
            genres
                .iter()
                .take(2)
                .copied()
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    parts.retain(|p| !p.is_empty());
    let hero = HeroItem {
        shown: true,
        eyebrow: row_title.into(),
        title: meta.name.as_str().into(),
        meta: parts.join(" · ").into(),
        rating: meta.rating().unwrap_or_default().into(),
        plot: meta.description.as_deref().unwrap_or_default().into(),
        left: SharedString::new(),
        progress: 0.0,
        play_label: "Play".into(),
        tint: text::tint(&meta.id),
        backdrop: Image::default(),
        has_backdrop: false,
    };
    (hero, meta.background.clone())
}

/// The title of the video `item` is at, from its metadata.
fn video_title(meta: &MetaItem, item: &LibraryItem) -> Option<String> {
    let video = item.state.video_id.as_deref()?;
    meta.videos
        .iter()
        .find(|v| v.id == video)
        .map(|v| v.title().to_owned())
        .filter(|t| !t.is_empty())
}

fn episode_plot<'a>(meta: &'a MetaItem, video: &str) -> Option<&'a str> {
    meta.videos
        .iter()
        .find(|v| v.id == video)
        .map(|v| v.overview())
        .filter(|o| !o.is_empty())
}

/// A row's title: the catalog's, with the type's heading unless it says
/// it already, for example `Trending movies`.
fn row_title(state: &super::State, at: CatalogRef) -> String {
    let Some((_, catalog)) = state.sources.catalog(at) else {
        return String::new();
    };
    let heading = state
        .sections
        .iter()
        .find(|s| s.kind == catalog.kind)
        .map(|s| s.title.to_lowercase())
        .unwrap_or_default();
    let title = catalog.title();
    let lower = title.to_lowercase();
    if heading.is_empty()
        || ["movie", "series", "show", "anime"]
            .iter()
            .any(|w| lower.contains(w))
    {
        title.to_owned()
    } else {
        format!("{title} {heading}")
    }
}

/// Fetched metadata, by title id.
pub(super) type Metas = HashMap<String, MetaItem>;
