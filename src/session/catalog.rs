//! A catalog, full width: its first page, more pages as the grid nears its
//! end (Stremio's `skip`), a genre filter, and posters kept only near the
//! screen.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use anchor::addon::MetaPreview;
use anchor::sources::CatalogRef;
use slint::{Image, Model, ModelRc, VecModel};

use super::home::poster_item;
use super::{Session, spawn};
use crate::art::Size;
use crate::ui::{PosterItem, Screen};

/// The catalog showing.
#[derive(Default)]
pub(super) struct Browse {
    at: Option<CatalogRef>,
    genre: Option<String>,
    metas: Vec<MetaPreview>,
    items: Rc<VecModel<PosterItem>>,
    loading: bool,
    /// The catalog said all it has.
    exhausted: bool,
    /// Bumped for each catalog or genre: older pages are dropped.
    generation: u64,
    /// Items holding a poster, and those allowed to.
    shown: HashSet<usize>,
    window: Range<usize>,
    /// The titles last reported on screen, as first and count.
    visible: (usize, usize),
}

impl Session {
    /// A catalog picked in the sidebar: the first page of it, every genre.
    pub(super) fn open_catalog(self: &Rc<Self>, section: i32, catalog: i32) {
        let at = {
            let state = self.state.borrow();
            usize::try_from(section)
                .ok()
                .and_then(|s| state.sections.get(s))
                .and_then(|s| s.catalogs.get(usize::try_from(catalog).ok()?))
                .copied()
        };
        let Some(at) = at else {
            return;
        };
        self.light_catalog(section, catalog);
        self.browse(at, None);
    }

    /// "See all" on Home's row `row`.
    pub(super) fn see_all(self: &Rc<Self>, row: i32) {
        let found = {
            let state = self.state.borrow();
            let at = usize::try_from(row)
                .ok()
                .and_then(|r| state.sources.browsable().get(r).copied());
            at.and_then(|at| {
                state.sections.iter().enumerate().find_map(|(s, section)| {
                    let c = section.catalogs.iter().position(|c| *c == at)?;
                    Some((s, c))
                })
            })
        };
        if let Some((s, c)) = found {
            self.open_catalog(s as i32, c as i32);
        }
    }

    /// A genre chip: the catalog again, filtered; -1 for every genre.
    pub(super) fn catalog_genre_selected(self: &Rc<Self>, genre: i32) {
        let (at, name) = {
            let state = self.state.borrow();
            let Some(at) = state.browse.at else {
                return;
            };
            let name = usize::try_from(genre).ok().and_then(|g| {
                state
                    .sources
                    .catalog(at)
                    .and_then(|(_, c)| c.genres().get(g).cloned())
            });
            (at, name)
        };
        self.browse(at, name);
    }

    /// Titles `first` to `first + count` are on screen: their posters, and a
    /// screen either side, are kept; the next page is asked for when the
    /// end is near.
    pub(super) fn catalog_items_visible(self: &Rc<Self>, first: i32, count: i32) {
        let (urls, more) = {
            let mut state = self.state.borrow_mut();
            let browse = &mut state.browse;
            let len = browse.metas.len();
            let count = usize::try_from(count).unwrap_or(0);
            let first = usize::try_from(first).unwrap_or(0);
            browse.visible = (first, count);
            let first = first.min(len);
            let end = (first + count).min(len);
            browse.window = first.saturating_sub(count)..(end + count).min(len);
            let far: Vec<usize> = browse
                .shown
                .iter()
                .copied()
                .filter(|i| !browse.window.contains(i))
                .collect();
            for i in far {
                browse.shown.remove(&i);
                let mut item = browse.items.row_data(i).expect("shown items exist");
                item.poster = Image::default();
                item.has_poster = false;
                browse.items.set_row_data(i, item);
            }
            let urls: Vec<String> = browse
                .window
                .clone()
                .filter_map(|i| browse.metas[i].poster.clone())
                .collect();
            // A screen of titles or less left: the next page.
            let more = !browse.loading && !browse.exhausted && end + count >= len && len > 0;
            (urls, more)
        };
        let mut art = self.art.borrow_mut();
        for url in &urls {
            art.request(url, Size::Poster);
        }
        drop(art);
        if more {
            self.load_page();
        }
    }

    /// A poster for the grid arrived.
    pub(super) fn catalog_art_ready(&self, url: &str, image: &Image) {
        let mut state = self.state.borrow_mut();
        let browse = &mut state.browse;
        for i in browse.window.clone() {
            if browse.metas[i].poster.as_deref() == Some(url) {
                let mut item = browse.items.row_data(i).expect("in range");
                item.poster = image.clone();
                item.has_poster = true;
                browse.items.set_row_data(i, item);
                browse.shown.insert(i);
            }
        }
    }
}

impl Session {
    /// Title `i` of the grid.
    pub(super) fn catalog_preview(&self, i: i32) -> Option<MetaPreview> {
        let state = self.state.borrow();
        state.browse.metas.get(usize::try_from(i).ok()?).cloned()
    }
}

// Private
impl Session {
    /// Shows catalog `at`, filtered by `genre`, from its first page.
    fn browse(self: &Rc<Self>, at: CatalogRef, genre: Option<String>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (title, source, genres, genre_index) = {
            let state = self.state.borrow();
            let Some((addon, catalog)) = state.sources.catalog(at) else {
                return;
            };
            let heading = state
                .sections
                .iter()
                .find(|s| s.kind == catalog.kind)
                .map_or_else(|| catalog.kind.clone(), |s| s.title.clone());
            let genres: Vec<slint::SharedString> =
                catalog.genres().iter().map(Into::into).collect();
            let index = genre
                .as_ref()
                .and_then(|g| catalog.genres().iter().position(|n| n == g))
                .map_or(-1, |i| i as i32);
            (
                catalog.title().to_owned(),
                format!("{heading} · {}", addon.manifest.name),
                genres,
                index,
            )
        };
        let items = Rc::new(VecModel::default());
        {
            let mut state = self.state.borrow_mut();
            let browse = &mut state.browse;
            browse.generation += 1;
            browse.at = Some(at);
            browse.genre = genre;
            browse.metas.clear();
            browse.items = Rc::clone(&items);
            browse.loading = false;
            browse.exhausted = false;
            browse.shown.clear();
            browse.window = 0..0;
        }
        app.set_catalog_title(title.into());
        app.set_catalog_source(source.into());
        app.set_catalog_genres(ModelRc::new(VecModel::from(genres)));
        app.set_catalog_genre(genre_index);
        app.set_catalog_items(ModelRc::from(items));
        app.set_catalog_note("Loading…".into());
        app.set_back_label("".into());
        self.show(Screen::Catalog);
        app.invoke_rewind_catalog();
        self.load_page();
    }

    /// Asks for the page after the titles loaded so far.
    fn load_page(self: &Rc<Self>) {
        let (addon, catalog, genre, skip, generation) = {
            let mut state = self.state.borrow_mut();
            let Some(at) = state.browse.at else {
                return;
            };
            let Some((addon, catalog)) = state
                .sources
                .catalog(at)
                .map(|(a, c)| (a.clone(), c.clone()))
            else {
                return;
            };
            let browse = &mut state.browse;
            browse.loading = true;
            (
                addon,
                catalog,
                browse.genre.clone(),
                browse.metas.len(),
                browse.generation,
            )
        };
        if let Some(app) = self.app.upgrade() {
            app.set_catalog_loading_more(skip > 0);
        }
        let addons = self.addons.clone();
        spawn(
            move || {
                let skip = skip.to_string();
                let mut extra: Vec<(&str, &str)> = Vec::new();
                if let Some(genre) = &genre {
                    extra.push(("genre", genre));
                }
                if skip != "0" {
                    extra.push(("skip", &skip));
                }
                addons.catalog(&addon, &catalog, &extra)
            },
            move |s, result| s.page_loaded(generation, result),
        );
    }

    fn page_loaded(
        self: &Rc<Self>,
        generation: u64,
        result: Result<Vec<MetaPreview>, anchor::net::Error>,
    ) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let note = {
            let mut state = self.state.borrow_mut();
            if state.browse.generation != generation {
                return;
            }
            let library = state.library.clone();
            let browse = &mut state.browse;
            browse.loading = false;
            match result {
                Ok(page) => {
                    let known: HashSet<String> =
                        browse.metas.iter().map(|m| m.id.clone()).collect();
                    let new: Vec<MetaPreview> = page
                        .into_iter()
                        .filter(|m| !known.contains(&m.id))
                        .collect();
                    if new.is_empty() {
                        browse.exhausted = true;
                    }
                    for meta in &new {
                        browse.items.push(poster_item(meta, &library));
                    }
                    browse.metas.extend(new);
                    if browse.metas.is_empty() {
                        "Nothing here yet.".to_owned()
                    } else {
                        String::new()
                    }
                }
                Err(e) => {
                    browse.exhausted = true;
                    if browse.metas.is_empty() {
                        format!("Could not load this catalog: {e}.")
                    } else {
                        String::new()
                    }
                }
            }
        };
        app.set_catalog_note(note.into());
        app.set_catalog_loading_more(false);
        // Ask for the posters now on screen, and maybe the next page.
        let (first, count) = self.state.borrow().browse.visible;
        self.catalog_items_visible(first as i32, count as i32);
    }
}
