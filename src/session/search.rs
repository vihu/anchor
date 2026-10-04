//! Search: every catalog that can search is asked once typing pauses; the
//! results drop down under the top bar's field, grouped by type and addon,
//! and fill the Search screen.

use std::rc::Rc;
use std::time::Duration;

use anchor::addon::MetaPreview;
use anchor::sources::CatalogRef;
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, TimerMode, VecModel};

use super::{Session, spawn, with_session};
use crate::art::Size;
use crate::text;
use crate::ui::{Screen, SearchData, SearchItem};

/// How long typing must pause before the catalogs are asked.
const PAUSE: Duration = Duration::from_millis(300);
/// Results kept per catalog.
const PER_CATALOG: usize = 20;

/// The search showing.
#[derive(Default)]
pub(super) struct Search {
    query: String,
    /// Each catalog's answer, in the account's order; `None` while asked.
    answers: Vec<(CatalogRef, Option<Vec<MetaPreview>>)>,
    /// The rows: a heading, or a title by catalog and place.
    rows: Vec<Option<(usize, usize)>>,
    items: Rc<VecModel<SearchItem>>,
    /// Bumped for each query: older answers are dropped.
    generation: u64,
}

impl Session {
    /// The field changed: asks again once typing pauses.
    pub(super) fn search_edited(self: &Rc<Self>) {
        self.search_timer.start(TimerMode::SingleShot, PAUSE, || {
            with_session(|s| s.run_search());
        });
    }

    /// Up or Down in the results: the next title, skipping headings.
    pub(super) fn search_move(&self, delta: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let data = app.global::<SearchData>();
        let rows = &self.state.borrow().search.rows;
        let mut index = data.get_index();
        loop {
            index += delta.signum();
            let Ok(at) = usize::try_from(index) else {
                // Above the first title: none chosen, so Enter shows all.
                data.set_index(-1);
                return;
            };
            match rows.get(at) {
                Some(Some(_)) => break,
                Some(None) => {}
                None => return,
            }
        }
        data.set_index(index);
        app.invoke_reveal_search_row();
    }

    /// "All results", or Enter with none chosen: the Search screen.
    pub(super) fn search_all(&self) {
        self.light_catalog(-1, -1);
        if let Some(app) = self.app.upgrade() {
            app.set_back_label("Home".into());
            app.set_screen(Screen::Search);
        }
    }

    /// A poster for the results arrived.
    pub(super) fn search_art_ready(&self, url: &str, image: &Image) {
        let state = self.state.borrow();
        let search = &state.search;
        for (row, at) in search.rows.iter().enumerate() {
            let Some((catalog, i)) = at else {
                continue;
            };
            let poster = search.answers[*catalog]
                .1
                .as_ref()
                .and_then(|m| m.get(*i))
                .and_then(|m| m.poster.as_deref());
            if poster == Some(url) {
                let mut item = search.items.row_data(row).expect("a model row per row");
                item.poster = image.clone();
                item.has_poster = true;
                search.items.set_row_data(row, item);
            }
        }
    }
}

// Private
impl Session {
    /// Asks every searchable catalog for the query in the field.
    fn run_search(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let data = app.global::<SearchData>();
        let query = data.get_query().trim().to_owned();
        let (refs, generation) = {
            let mut state = self.state.borrow_mut();
            if query == state.search.query {
                return;
            }
            let refs = if query.is_empty() {
                Vec::new()
            } else {
                state.sources.searchable()
            };
            let search = &mut state.search;
            search.generation += 1;
            search.query.clone_from(&query);
            search.answers = refs.iter().map(|at| (*at, None)).collect();
            (refs, search.generation)
        };
        data.set_index(-1);
        self.show_results();
        for (c, at) in refs.into_iter().enumerate() {
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
            let query = query.clone();
            spawn(
                move || addons.catalog(&addon, &catalog, &[("search", &query)]),
                move |s, result| {
                    {
                        let mut state = s.state.borrow_mut();
                        if state.search.generation != generation {
                            return;
                        }
                        let mut metas = result.unwrap_or_default();
                        metas.truncate(PER_CATALOG);
                        state.search.answers[c].1 = Some(metas);
                    }
                    s.show_results();
                },
            );
        }
    }

    /// Fills the results from the answers so far, and asks for posters.
    fn show_results(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let data = app.global::<SearchData>();
        let (urls, note) = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let mut rows = Vec::new();
            let mut items = Vec::new();
            let mut urls = Vec::new();
            for (c, (at, answer)) in state.search.answers.iter().enumerate() {
                let Some(metas) = answer.as_ref().filter(|m| !m.is_empty()) else {
                    continue;
                };
                let Some((addon, catalog)) = state.sources.catalog(*at) else {
                    continue;
                };
                let heading = state
                    .sections
                    .iter()
                    .find(|s| s.kind == catalog.kind)
                    .map_or_else(|| catalog.kind.clone(), |s| s.title.clone());
                rows.push(None);
                items.push(SearchItem {
                    heading: true,
                    title: format!("{heading} · {}", addon.manifest.name).into(),
                    ..SearchItem::default()
                });
                let kind = match catalog.kind.as_str() {
                    "movie" => "Movie",
                    "series" => "Series",
                    _ => heading.as_str(),
                };
                for (i, meta) in metas.iter().enumerate() {
                    rows.push(Some((c, i)));
                    items.push(result_item(meta, kind, &addon.manifest.name));
                    urls.extend(meta.poster.clone());
                }
            }
            let waiting = state.search.answers.iter().any(|(_, a)| a.is_none());
            let note = if state.search.query.is_empty() {
                ""
            } else if waiting {
                "Searching…"
            } else if rows.is_empty() {
                "Nothing found."
            } else {
                ""
            };
            state.search.rows = rows;
            state.search.items = Rc::new(VecModel::from(items));
            data.set_items(ModelRc::from(Rc::clone(&state.search.items)));
            (urls, note)
        };
        data.set_note(SharedString::from(note));
        let mut art = self.art.borrow_mut();
        for url in &urls {
            art.request(url, Size::Poster);
        }
    }
}

/// A result: the title, then for example `Movie · 2025 · AIOMetadata`.
fn result_item(meta: &MetaPreview, kind: &str, addon: &str) -> SearchItem {
    let mut subtitle = vec![kind.to_owned()];
    let year = text::year(meta.release_info.as_deref());
    if !year.is_empty() {
        subtitle.push(year);
    }
    subtitle.push(addon.to_owned());
    SearchItem {
        heading: false,
        title: meta.name.as_str().into(),
        subtitle: subtitle.join(" · ").into(),
        detail: meta.rating().unwrap_or_default().into(),
        tint: text::tint(&meta.id),
        poster: Image::default(),
        has_poster: false,
    }
}
