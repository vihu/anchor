//! A title's page: what the catalog said at once, then its metadata; for a
//! series, its seasons and episodes with watched marks and the episode to
//! watch next selected. Mark watched writes to the library.

use std::rc::Rc;

use anchor::addon::{CastMember, MetaItem, MetaPreview, Video};
use anchor::library::{self, LibraryItem};
use jiff::Timestamp;
use slint::{Image, Model, ModelRc, SharedString, VecModel};

use super::details::{cast, cast_item, facts};
use super::streams::{Target, label};
use super::{Session, State};
use crate::art::Size;
use crate::text;
use crate::ui::{CastItem, EpisodeItem, Screen, SeasonItem, TitleDetails};

/// Cast members named on a page.
const CAST: usize = 3;

/// The page showing.
pub(super) struct Page {
    preview: MetaPreview,
    meta: Option<MetaItem>,
    /// The seasons in tab order (specials last), and the one showing.
    seasons: Vec<u32>,
    season: usize,
    /// The episodes of the season showing, in order.
    episodes: Vec<Video>,
    episode_model: Rc<VecModel<EpisodeItem>>,
    /// A movie's cast, a row each.
    cast: Vec<CastMember>,
    cast_model: Rc<VecModel<CastItem>>,
    /// Where the back button goes, and what it says.
    origin: (Screen, String),
    /// The pictures asked for, so a refresh keeps what already shows.
    backdrop: Option<String>,
    poster: Option<String>,
}

impl Page {
    /// The title's id.
    pub(super) fn id(&self) -> &str {
        &self.preview.id
    }

    /// What Play plays: the movie, or episode `episode` of the season
    /// showing; `None` before a series' episodes are known, or for one not
    /// out yet.
    pub(super) fn target(&self, episode: i32, now: Timestamp) -> Option<Target> {
        let preview = self.meta.as_ref().map_or(&self.preview, |m| &m.preview);
        let year = text::year(preview.release_info.as_deref());
        // A series' episodes are known only from its metadata.
        if self.meta.is_none() && preview.kind != "movie" {
            return None;
        }
        if self.seasons.is_empty() {
            let video_id = self
                .meta
                .as_ref()
                .map_or_else(|| self.preview.id.clone(), MetaItem::movie_video_id);
            return Some(Target {
                kind: preview.kind.clone(),
                meta_id: preview.id.clone(),
                video_id,
                name: preview.name.clone(),
                code: None,
                label: year,
                picture: preview
                    .background
                    .clone()
                    .or_else(|| preview.poster.clone()),
            });
        }
        let video = self.episodes.get(usize::try_from(episode).ok()?)?;
        if !released(video, now) {
            return None;
        }
        let code = text::code(video.season.unwrap_or(0), video.episode.unwrap_or(0));
        Some(Target {
            kind: preview.kind.clone(),
            meta_id: preview.id.clone(),
            video_id: video.id.clone(),
            name: preview.name.clone(),
            label: label(Some(&code), video.title(), &year),
            code: Some(code),
            picture: video
                .thumbnail
                .clone()
                .or_else(|| preview.background.clone()),
        })
    }
}

impl Session {
    /// The episode list again: watched marks, progress, what plays.
    pub(super) fn refresh_episodes(&self) {
        self.show_season(false);
    }

    /// Opens the page of `preview`, which may be all a catalog knows; its
    /// metadata follows.
    pub(super) fn open_title(self: &Rc<Self>, preview: MetaPreview) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let screen = app.get_screen();
        let origin = match screen {
            Screen::Catalog => (Screen::Catalog, app.get_catalog_title().to_string()),
            Screen::Search => (Screen::Search, "Search".to_owned()),
            Screen::Title => self
                .state
                .borrow()
                .page
                .as_ref()
                .map_or((Screen::Home, "Home".to_owned()), |p| p.origin.clone()),
            _ => (Screen::Home, "Home".to_owned()),
        };
        let (kind, id) = (preview.kind.clone(), preview.id.clone());
        let known = self.state.borrow().metas.get(&id).cloned();
        self.state.borrow_mut().page = Some(Page {
            preview,
            meta: None,
            seasons: Vec::new(),
            season: 0,
            episodes: Vec::new(),
            episode_model: Rc::default(),
            cast: Vec::new(),
            cast_model: Rc::default(),
            origin: origin.clone(),
            backdrop: None,
            poster: None,
        });
        app.set_back_label(origin.1.as_str().into());
        app.set_title_episode_index(-1);
        // The panel belonged to the page before.
        app.set_streams_open(false);
        self.light_catalog(-1, -1);
        match known {
            Some(meta) => self.page_meta_ready(&meta),
            None => {
                self.refresh_page();
                self.fetch_meta(&kind, &id);
            }
        }
        self.show(Screen::Title);
    }

    /// The top bar's back button, or Esc on the page: where it came from.
    pub(super) fn title_back(self: &Rc<Self>) {
        let origin = self.state.borrow_mut().page.take().map(|p| p.origin);
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_streams_open(false);
        match origin {
            Some((Screen::Catalog, _)) => {
                app.set_back_label(SharedString::new());
                self.show(Screen::Catalog);
            }
            Some((Screen::Search, _)) => {
                app.set_back_label("Home".into());
                self.show(Screen::Search);
            }
            _ => self.navigate(super::shell::HOME),
        }
    }

    /// A title's metadata arrived: the page takes it, if it shows it.
    pub(super) fn page_meta_ready(self: &Rc<Self>, meta: &MetaItem) {
        let library = self.state.borrow().library.clone();
        {
            let mut state = self.state.borrow_mut();
            let Some(page) = state
                .page
                .as_mut()
                .filter(|p| p.preview.id == meta.preview.id)
            else {
                return;
            };
            page.meta = Some(meta.clone());
            page.seasons = seasons(meta);
            let item = library.iter().find(|i| i.id == meta.preview.id);
            page.season = start_season(meta, &page.seasons, item);
        }
        self.show_season(true);
        self.refresh_page();
    }

    /// A season tab.
    pub(super) fn title_season_selected(self: &Rc<Self>, season: i32) {
        let Ok(season) = usize::try_from(season) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let Some(page) = state.page.as_mut().filter(|p| season < p.seasons.len()) else {
                return;
            };
            page.season = season;
        }
        self.show_season(true);
    }

    /// W or the button: the movie, or the selected episode, is watched or
    /// not.
    pub(super) fn title_toggle_watched(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let now = library::now();
        let item = {
            let state = self.state.borrow();
            let Some(page) = state.page.as_ref() else {
                return;
            };
            let mut item = library_item(&state, page, now);
            match &page.meta {
                Some(meta) if !page.episodes.is_empty() => {
                    let Some(video) = usize::try_from(app.get_title_episode_index())
                        .ok()
                        .and_then(|i| page.episodes.get(i))
                    else {
                        return;
                    };
                    let ids = meta.bitfield_ids();
                    let watched = item.watched_videos(ids.clone()).get(&video.id);
                    item.mark_video_watched(video, !watched, &ids, now);
                }
                _ => {
                    let watched = item.watched();
                    item.mark_watched(!watched, now);
                }
            }
            item
        };
        self.save_item(item);
        self.show_season(false);
        self.refresh_page();
        self.refresh_home();
    }

    /// A picture for the page arrived.
    pub(super) fn page_art_ready(&self, url: &str, size: Size, image: &Image) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let state = self.state.borrow();
        let Some(page) = state.page.as_ref() else {
            return;
        };
        match size {
            Size::Backdrop if page.backdrop.as_deref() == Some(url) => {
                let mut details = app.get_title_details();
                details.backdrop = image.clone();
                details.has_backdrop = true;
                app.set_title_details(details);
            }
            Size::Poster if page.poster.as_deref() == Some(url) => {
                let mut details = app.get_title_details();
                details.poster = image.clone();
                details.has_poster = true;
                app.set_title_details(details);
            }
            Size::Face => {
                for (i, person) in page.cast.iter().enumerate() {
                    if person.photo.as_deref() == Some(url) {
                        let mut item = page.cast_model.row_data(i).expect("a row per person");
                        item.photo = image.clone();
                        item.has_photo = true;
                        page.cast_model.set_row_data(i, item);
                    }
                }
            }
            Size::Still => {
                for (i, video) in page.episodes.iter().enumerate() {
                    if video.thumbnail.as_deref() == Some(url) {
                        let mut item = page.episode_model.row_data(i).expect("a row per episode");
                        item.still = image.clone();
                        item.has_still = true;
                        page.episode_model.set_row_data(i, item);
                    }
                }
            }
            _ => {}
        }
    }

    /// Fills the page's details: title, metadata line, plot, credits, the
    /// play button, and the backdrop or poster.
    pub(super) fn refresh_page(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (details, series, urls, cast) = {
            let mut state = self.state.borrow_mut();
            let library = state.library.clone();
            let Some(page) = state.page.as_mut() else {
                return;
            };
            let meta = page.meta.as_ref();
            let preview = meta.map_or(&page.preview, |m| &m.preview);
            let item = library.iter().find(|i| i.id == preview.id);
            let series = !page.seasons.is_empty();
            let backdrop = preview
                .background
                .clone()
                .or_else(|| page.preview.background.clone());
            let poster = preview
                .poster
                .clone()
                .or_else(|| page.preview.poster.clone());
            let current = app.get_title_details();
            let same_backdrop = backdrop.is_some() && backdrop == page.backdrop;
            let same_poster = poster.is_some() && poster == page.poster;
            // On the same page a new picture replaces the one showing when it
            // arrives; a new page starts without.
            let keep_backdrop = same_backdrop || page.backdrop.is_some();
            let keep_poster = same_poster || page.poster.is_some();
            page.backdrop.clone_from(&backdrop);
            page.poster.clone_from(&poster);
            // A movie shows its cast and details below; the credits
            // line names them otherwise.
            let movie = !series && preview.kind == "movie";
            let facts = if movie { facts(preview) } else { Vec::new() };
            let awards = preview.awards.clone().filter(|_| movie).unwrap_or_default();
            let people = if movie { cast(preview) } else { Vec::new() };
            let below = !facts.is_empty() || !awards.is_empty() || !people.is_empty();
            let photos = if people == page.cast {
                Vec::new()
            } else {
                page.cast_model
                    .set_vec(people.iter().map(cast_item).collect::<Vec<_>>());
                page.cast = people;
                page.cast.iter().filter_map(|p| p.photo.clone()).collect()
            };
            let resume = meta
                .filter(|_| !series)
                .and_then(|m| item.and_then(|i| i.resume_at(&m.movie_video_id())));
            let details = TitleDetails {
                title: preview.name.as_str().into(),
                meta: meta_line(preview, meta, series).into(),
                rating: preview.rating().unwrap_or_default().into(),
                plot: preview.description.as_deref().unwrap_or_default().into(),
                credits: if below {
                    SharedString::new()
                } else {
                    credits(preview).into()
                },
                cert: preview
                    .app_extras
                    .certification
                    .as_deref()
                    .unwrap_or_default()
                    .into(),
                facts: ModelRc::new(VecModel::from(facts)),
                awards: awards.into(),
                tint: text::tint(&preview.id),
                poster: if keep_poster {
                    current.poster
                } else {
                    Image::default()
                },
                has_poster: keep_poster && current.has_poster,
                backdrop: if keep_backdrop {
                    current.backdrop
                } else {
                    Image::default()
                },
                has_backdrop: keep_backdrop && current.has_backdrop,
                note: if meta.is_none() {
                    "Loading…".into()
                } else {
                    SharedString::new()
                },
                play_label: if resume.is_some() {
                    "Resume".into()
                } else {
                    "Play".into()
                },
                left: item
                    .filter(|_| resume.is_some())
                    .map(|i| text::left(i.state.time_offset, i.state.duration))
                    .unwrap_or_default()
                    .into(),
                restart: resume.is_some(),
                watched: item.is_some_and(LibraryItem::watched),
                playing: current.playing,
            };
            let urls = (
                backdrop.filter(|_| !same_backdrop),
                poster.filter(|_| !same_poster),
            );
            (details, series, urls, (Rc::clone(&page.cast_model), photos))
        };
        app.set_title_details(details);
        app.set_title_series(series);
        app.set_title_cast(ModelRc::from(cast.0));
        let mut art = self.art.borrow_mut();
        for url in &cast.1 {
            art.request(url, Size::Face);
        }
        if let Some(url) = urls.0 {
            art.request(&url, Size::Backdrop);
        }
        if let Some(url) = urls.1 {
            art.request(&url, Size::Poster);
        }
    }
}

// Private
impl Session {
    /// Fills the season tabs and the episodes of the season showing;
    /// `select` picks the episode to watch next.
    fn show_season(&self, select: bool) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let now = library::now();
        let (tabs, season, model, pick, stills) = {
            let mut state = self.state.borrow_mut();
            let library = state.library.clone();
            let state_playing = state.playing.as_ref().map(|p| p.video_id().to_owned());
            let Some(page) = state.page.as_mut() else {
                return;
            };
            let Some(meta) = page.meta.as_ref() else {
                return;
            };
            let item = library.iter().find(|i| i.id == meta.preview.id);
            let watched = item.map(|i| i.watched_videos(meta.bitfield_ids()));
            let playing_video = state_playing.clone();
            let shown: Vec<Video> = meta
                .display_videos()
                .into_iter()
                .filter(|v| v.season == page.seasons.get(page.season).copied())
                .cloned()
                .collect();
            let tabs: Vec<SeasonItem> = page
                .seasons
                .iter()
                .map(|s| SeasonItem {
                    name: if *s == 0 {
                        "Specials".into()
                    } else {
                        format!("Season {s}").into()
                    },
                    count: meta
                        .videos
                        .iter()
                        .filter(|v| v.season == Some(*s))
                        .count()
                        .to_string()
                        .into(),
                })
                .collect();
            let items: Vec<EpisodeItem> = shown
                .iter()
                .map(|v| {
                    let mut episode =
                        episode_item(v, item, watched.as_ref().is_some_and(|w| w.get(&v.id)), now);
                    if playing_video.as_deref() == Some(v.id.as_str()) {
                        episode.playing = true;
                        episode.state = "Playing in mpv".into();
                    }
                    episode
                })
                .collect();
            let pick = next_episode(&shown, item, watched.as_ref(), now);
            let stills: Vec<String> = shown.iter().filter_map(|v| v.thumbnail.clone()).collect();
            page.episodes = shown;
            page.episode_model = Rc::new(VecModel::from(items));
            (
                tabs,
                page.season,
                Rc::clone(&page.episode_model),
                pick,
                stills,
            )
        };
        app.set_title_seasons(ModelRc::new(VecModel::from(tabs)));
        app.set_title_season_index(season as i32);
        app.set_title_episodes(ModelRc::from(model));
        if select || app.get_title_episode_index() < 0 {
            app.set_title_episode_index(pick as i32);
        }
        app.invoke_reveal_episode();
        let mut art = self.art.borrow_mut();
        for url in &stills {
            art.request(url, Size::Still);
        }
    }
}

/// The library item for the page's title: the account's, or a new one.
fn library_item(state: &State, page: &Page, now: Timestamp) -> LibraryItem {
    let preview = page.meta.as_ref().map_or(&page.preview, |m| &m.preview);
    state
        .library
        .iter()
        .find(|i| i.id == preview.id)
        .cloned()
        .unwrap_or_else(|| LibraryItem::new(preview, now))
}

/// The seasons of a title in tab order: regular ones, then specials; none
/// for a title without seasons.
fn seasons(meta: &MetaItem) -> Vec<u32> {
    let mut seasons: Vec<u32> = Vec::new();
    for video in meta.display_videos() {
        if let Some(season) = video.season
            && !seasons.contains(&season)
        {
            seasons.push(season);
        }
    }
    seasons
}

/// The season a series opens on: that of the episode the user is at, else
/// the first.
fn start_season(meta: &MetaItem, seasons: &[u32], item: Option<&LibraryItem>) -> usize {
    item.and_then(|i| i.state.video_id.as_deref())
        .and_then(|id| meta.videos.iter().find(|v| v.id == id))
        .and_then(|v| v.season)
        .and_then(|s| seasons.iter().position(|x| *x == s))
        .unwrap_or(0)
}

/// The episode to select: the one the user is at, else the first not
/// watched and out, else the first.
fn next_episode(
    episodes: &[Video],
    item: Option<&LibraryItem>,
    watched: Option<&anchor::watched::Watched>,
    now: Timestamp,
) -> usize {
    let current = item.and_then(|i| i.state.video_id.as_deref());
    episodes
        .iter()
        .position(|v| Some(v.id.as_str()) == current)
        .or_else(|| {
            episodes
                .iter()
                .position(|v| !watched.is_some_and(|w| w.get(&v.id)) && released(v, now))
        })
        .unwrap_or(0)
}

fn released(video: &Video, now: Timestamp) -> bool {
    video
        .released_ms()
        .is_none_or(|ms| ms <= now.as_millisecond())
}

fn episode_item(
    video: &Video,
    item: Option<&LibraryItem>,
    watched: bool,
    now: Timestamp,
) -> EpisodeItem {
    let (season, number) = (video.season.unwrap_or(0), video.episode.unwrap_or(0));
    let resume = item.and_then(|i| i.resume_at(&video.id)).is_some();
    let left = item
        .filter(|_| resume)
        .map(|i| text::left(i.state.time_offset, i.state.duration))
        .unwrap_or_default();
    let progress = item.filter(|_| resume).map_or(0.0, |i| i.progress() as f32);
    let upcoming = !released(video, now);
    let state = if upcoming {
        video
            .released
            .as_deref()
            .and_then(|r| r.parse::<Timestamp>().ok())
            .map(|t| format!("Out {}", t.strftime("%a %-d %b")))
            .unwrap_or_default()
    } else if resume {
        left.clone()
    } else if watched {
        "Watched".to_owned()
    } else {
        String::new()
    };
    EpisodeItem {
        number: number.to_string().into(),
        code: text::code(season, number).into(),
        left: left.into(),
        title: video.title().into(),
        plot: video.overview().into(),
        state: state.into(),
        still: Image::default(),
        has_still: false,
        progress,
        watched,
        resumable: resume,
        upcoming,
        playing: false,
    }
}

/// For example `2025 · 2 h 4 min · Adventure, Family`, or for a series
/// `2024 · 3 seasons · Comedy, Family`.
fn meta_line(preview: &MetaPreview, meta: Option<&MetaItem>, series: bool) -> String {
    let mut parts = vec![text::year(preview.release_info.as_deref())];
    if series {
        let count = meta.map_or(0, |m| seasons(m).iter().filter(|s| **s != 0).count());
        if count > 0 {
            parts.push(if count == 1 {
                "1 season".to_owned()
            } else {
                format!("{count} seasons")
            });
        }
    } else if let Some(runtime) = &preview.runtime {
        parts.push(text::runtime(runtime));
    }
    let genres = preview.genre_names();
    if !genres.is_empty() {
        parts.push(
            genres
                .iter()
                .take(3)
                .copied()
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

/// For example `Directed by Ana Dimas · With Teo Larsen, Margit Holm`.
fn credits(preview: &MetaPreview) -> String {
    let mut parts = Vec::new();
    let directors = preview.people("Directors");
    if !directors.is_empty() {
        parts.push(format!("Directed by {}", directors.join(", ")));
    }
    let cast: Vec<&str> = preview.people("Cast").into_iter().take(CAST).collect();
    if !cast.is_empty() {
        parts.push(format!("With {}", cast.join(", ")));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series() -> MetaItem {
        serde_json::from_value(serde_json::json!({
            "id": "tt1", "type": "series", "name": "Small Thieves", "releaseInfo": "2024-",
            "genres": ["Comedy", "Family"],
            "videos": [
                {"id": "tt1:0:1", "season": 0, "episode": 1, "released": "2024-01-01T00:00:00Z"},
                {"id": "tt1:1:1", "season": 1, "episode": 1, "released": "2024-01-01T00:00:00Z"},
                {"id": "tt1:1:2", "season": 1, "episode": 2, "released": "2024-01-08T00:00:00Z"},
                {"id": "tt1:2:1", "season": 2, "episode": 1, "released": "2025-01-01T00:00:00Z"},
                {"id": "tt1:2:2", "season": 2, "episode": 2, "released": "2099-01-01T00:00:00Z"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn seasons_with_specials_last_and_the_meta_line() {
        let meta = series();
        assert_eq!(seasons(&meta), [1, 2, 0]);
        assert_eq!(
            meta_line(&meta.preview, Some(&meta), true),
            "2024 · 2 seasons · Comedy, Family"
        );
    }

    #[test]
    fn the_next_episode_is_the_first_unwatched_one_out() {
        let meta = series();
        let now: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        let season_two: Vec<Video> = meta.videos[3..].to_vec();
        let mut watched = anchor::watched::Watched::none(meta.bitfield_ids());
        assert_eq!(next_episode(&season_two, None, Some(&watched), now), 0);
        watched.set("tt1:2:1", true);
        assert_eq!(
            next_episode(&season_two, None, Some(&watched), now),
            0,
            "the only other one is not out: back to the first"
        );
        let upcoming = episode_item(&season_two[1], None, false, now);
        assert!(upcoming.upcoming);
        assert_eq!(upcoming.state, "Out Thu 1 Jan");
    }

    #[test]
    fn credits_name_directors_then_cast() {
        let preview: MetaPreview = serde_json::from_value(serde_json::json!({
            "id": "tt9", "type": "movie", "name": "X",
            "links": [
                {"name": "Ana Dimas", "category": "Directors"},
                {"name": "A", "category": "Cast"}, {"name": "B", "category": "Cast"},
                {"name": "C", "category": "Cast"}, {"name": "D", "category": "Cast"}
            ]
        }))
        .unwrap();
        assert_eq!(credits(&preview), "Directed by Ana Dimas · With A, B, C");
    }
}
