//! The streams panel: every stream addon is asked for the video, and the
//! answers show in the account's order as the addons wrote them; the stream
//! last played is selected. Enter starts the user's mpv on the selected one.

use std::collections::HashSet;
use std::rc::Rc;

use anchor::addon::{Stream, Subtitle};
use anchor::library;
use anchor::player::{self, Launch, Playback};
use anchor::store::LastStream;
use camino::Utf8Path;
use slint::{ModelRc, SharedString, VecModel};

use super::playing::Playing;
use super::{Session, on_ui_thread, spawn};
use crate::ui::StreamItem;

/// Where a title starts playing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Start {
    /// Where it was left, when it was started and not finished.
    #[default]
    Resume,
    /// From the beginning.
    Beginning,
}

/// The video the panel is for, and how the window names it.
#[derive(Clone, Debug, Default)]
pub(super) struct Target {
    /// The title's type and id, for example `series` and `tt0903747`.
    pub(super) kind: String,
    pub(super) meta_id: String,
    /// The video, for example `tt0903747:2:4`.
    pub(super) video_id: String,
    /// The title's name.
    pub(super) name: String,
    /// For example `S2 E4`, for an episode.
    pub(super) code: Option<String>,
    /// For example `S2 E4 · The Pantry Job`, or the year of a movie.
    pub(super) label: String,
    /// A still or backdrop for the now-playing bar.
    pub(super) picture: Option<String>,
}

/// A stream addon's name and answer; `None` while it is asked.
type Answer = (String, Option<Result<Vec<Stream>, String>>);

/// The panel showing.
#[derive(Default)]
pub(super) struct Panel {
    target: Option<Target>,
    start: Start,
    /// Each stream addon's name and answer; `None` while asked.
    answers: Vec<Answer>,
    /// The rows: an addon's heading, or a stream by addon and place.
    rows: Vec<Option<(usize, usize)>>,
    /// Streams mpv could not play, by addon and place.
    failed: HashSet<(usize, usize)>,
    subtitles: Vec<Subtitle>,
    generation: u64,
}

impl Session {
    /// Enter or Play on a title's page: the streams for the movie or the
    /// selected episode.
    pub(super) fn title_play(self: &Rc<Self>, start: Start) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let target = {
            let state = self.state.borrow();
            let Some(page) = state.page.as_ref() else {
                return;
            };
            page.target(app.get_title_episode_index(), library::now())
        };
        let Some(target) = target else {
            return;
        };
        self.open_streams(target, start, None);
    }

    /// Asks every stream addon for `target`'s streams and shows the panel;
    /// `banner` says why it opened again, when playing failed.
    pub(super) fn open_streams(
        self: &Rc<Self>,
        target: Target,
        start: Start,
        banner: Option<String>,
    ) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (stream_addons, subtitle_addons, generation) = {
            let mut state = self.state.borrow_mut();
            let same = state
                .panel
                .target
                .as_ref()
                .is_some_and(|t| t.video_id == target.video_id);
            let streams: Vec<_> = state
                .sources
                .serving("stream", &target.kind, &target.video_id)
                .into_iter()
                .cloned()
                .collect();
            let subtitles: Vec<_> = if self.settings.borrow().addon_subtitles {
                state
                    .sources
                    .serving("subtitles", &target.kind, &target.video_id)
                    .into_iter()
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            let panel = &mut state.panel;
            panel.generation += 1;
            if !same {
                panel.failed.clear();
            }
            panel.answers = streams
                .iter()
                .map(|a| (a.manifest.name.clone(), None))
                .collect();
            panel.rows.clear();
            panel.subtitles.clear();
            panel.start = start;
            panel.target = Some(target.clone());
            (streams, subtitles, panel.generation)
        };
        app.set_streams_subtitle(target.label.as_str().into());
        app.set_streams_banner(banner.unwrap_or_default().into());
        app.set_stream_index(-1);
        app.set_streams_open(true);
        self.show_streams();
        for (a, addon) in stream_addons.into_iter().enumerate() {
            let addons = self.addons.clone();
            let (kind, id) = (target.kind.clone(), target.video_id.clone());
            spawn(
                move || {
                    addons
                        .streams(&addon, &kind, &id)
                        .map_err(|e| e.to_string())
                },
                move |s, result| {
                    {
                        let mut state = s.state.borrow_mut();
                        if state.panel.generation != generation {
                            return;
                        }
                        state.panel.answers[a].1 = Some(result);
                    }
                    s.show_streams();
                },
            );
        }
        for addon in subtitle_addons {
            let addons = self.addons.clone();
            let (kind, id) = (target.kind.clone(), target.video_id.clone());
            spawn(
                move || {
                    addons
                        .subtitles(&addon, &kind, &id, &[])
                        .unwrap_or_default()
                },
                move |s, subtitles| {
                    let mut state = s.state.borrow_mut();
                    if state.panel.generation == generation {
                        state.panel.subtitles.extend(subtitles);
                    }
                },
            );
        }
    }

    /// Esc or the close button.
    pub(super) fn close_streams(&self) {
        if let Some(app) = self.app.upgrade() {
            app.set_streams_open(false);
            app.invoke_focus_screen();
        }
    }

    /// Enter on a stream, or a click: plays it in mpv.
    pub(super) fn play_stream(self: &Rc<Self>, row: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let picked = {
            let state = self.state.borrow();
            let panel = &state.panel;
            usize::try_from(row)
                .ok()
                .and_then(|r| *panel.rows.get(r)?)
                .and_then(|(a, i)| {
                    let stream = panel.answers[a].1.as_ref()?.as_ref().ok()?.get(i)?.clone();
                    Some(((a, i), stream, panel.target.clone()?, panel.start))
                })
        };
        let Some((at, stream, target, start)) = picked else {
            return;
        };
        let Some(url) = stream.url.clone() else {
            return;
        };
        let settings = self.settings.borrow().clone();
        let configured = Some(Utf8Path::new(settings.mpv_path.as_str()));
        let Some(mpv) = player::find_mpv(configured.filter(|p| !p.as_str().is_empty())) else {
            app.set_streams_banner(
                "anchor plays through mpv, and cannot find it. Install it (pacman -S mpv, brew install mpv), or set where it is in Settings."
                    .into(),
            );
            return;
        };
        let Some(extra_args) = player::split_args(&settings.mpv_args) else {
            app.set_streams_banner(
                "The extra mpv arguments in Settings have a quote that is not closed.".into(),
            );
            return;
        };
        let resume = {
            let state = self.state.borrow();
            state
                .library
                .iter()
                .find(|i| i.id == target.meta_id)
                .and_then(|i| i.resume_at(&target.video_id))
        };
        let launch = Launch {
            url,
            // For example `Small Thieves · S2 E4 The Pantry Job`.
            title: match &target.code {
                Some(_) => format!("{} · {}", target.name, target.label.replacen(" · ", " ", 1)),
                None => target.name.clone(),
            },
            start: if start == Start::Resume { resume } else { None },
            subtitles: self.chosen_subtitles(&settings.subtitle_languages),
            headers: stream.headers(),
            extra_args,
        };
        self.remember_stream(&target.video_id, &stream);
        let token = self.next_token();
        let playback = match Playback::start(&mpv, &launch, move |event| {
            on_ui_thread(move |s| s.player_event(token, event));
        }) {
            Ok(playback) => playback,
            Err(e) => {
                app.set_streams_banner(format!("{e}").into());
                return;
            }
        };
        // A title already playing in another mpv is stopped.
        let previous = self.state.borrow_mut().playing.take();
        if let Some(previous) = previous {
            previous.stop();
        }
        self.state.borrow_mut().panel.failed.remove(&at);
        self.start_playing(Playing::new(token, playback, target, launch.start));
        app.set_streams_open(false);
        app.invoke_focus_screen();
    }

    /// mpv could not play the stream last played for `target`: the panel
    /// again, the stream marked, with why.
    pub(super) fn stream_failed(self: &Rc<Self>, target: Target, why: &str) {
        let start = {
            let mut state = self.state.borrow_mut();
            let last = self.paths.last_streams().get(&target.video_id).cloned();
            let panel = &mut state.panel;
            if let Some(at) = last.and_then(|last| find_stream(&panel.answers, &last)) {
                panel.failed.insert(at);
            }
            panel.start
        };
        self.open_streams(
            target,
            start,
            Some(format!(
                "mpv could not open this stream ({why}). Pick another one."
            )),
        );
    }
}

// Private
impl Session {
    /// Fills the panel from the answers so far, selecting the stream last
    /// played, or the first.
    fn show_streams(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let video = {
            let state = self.state.borrow();
            state.panel.target.as_ref().map(|t| t.video_id.clone())
        };
        let last = video.and_then(|v| self.paths.last_streams().remove(&v));
        let (items, select, note) = {
            let mut state = self.state.borrow_mut();
            let panel = &mut state.panel;
            let last_at = last.as_ref().and_then(|l| find_stream(&panel.answers, l));
            let mut rows = Vec::new();
            let mut items = Vec::new();
            for (a, (name, answer)) in panel.answers.iter().enumerate() {
                let (count, error) = match answer {
                    Some(Ok(streams)) => (Some(streams), None),
                    Some(Err(e)) => (None, Some(e.as_str())),
                    None => (None, None),
                };
                let playable: Vec<(usize, &Stream)> = count
                    .map(|s| {
                        s.iter()
                            .enumerate()
                            .filter(|(_, s)| s.url.is_some())
                            .collect()
                    })
                    .unwrap_or_default();
                let hidden = count.map_or(0, |s| s.len() - playable.len());
                let details = match (answer, error) {
                    (None, _) => "asking…".to_owned(),
                    (_, Some(e)) => format!("did not answer: {e}"),
                    _ => count_text(playable.len(), hidden),
                };
                rows.push(None);
                items.push(StreamItem {
                    heading: true,
                    name: name.as_str().into(),
                    details: details.into(),
                    ..StreamItem::default()
                });
                for (i, stream) in playable {
                    let failed = panel.failed.contains(&(a, i));
                    rows.push(Some((a, i)));
                    items.push(StreamItem {
                        heading: false,
                        name: stream.name.as_deref().unwrap_or(name).into(),
                        details: stream.details().into(),
                        tag: if failed {
                            "Failed".into()
                        } else if last_at == Some((a, i)) {
                            "Last used".into()
                        } else {
                            SharedString::new()
                        },
                        failed,
                    });
                }
            }
            let streams = rows.iter().flatten().count();
            let waiting = panel.answers.iter().any(|(_, a)| a.is_none());
            let note = if panel.answers.is_empty() {
                "None of your addons offers streams for this title."
            } else if streams == 0 && !waiting {
                "No streams for this one. It may be too new: try again later, or loosen the filters in your stream addon."
            } else {
                ""
            };
            let select = rows
                .iter()
                .position(|r| {
                    r.is_some()
                        && *r == last_at
                        && last_at.is_some_and(|at| !panel.failed.contains(&at))
                })
                .or_else(|| {
                    rows.iter()
                        .position(|r| r.is_some_and(|at| !panel.failed.contains(&at)))
                });
            panel.rows = rows;
            (items, select, note)
        };
        app.set_streams(ModelRc::new(VecModel::from(items)));
        app.set_streams_note(note.into());
        let current = app.get_stream_index();
        let still_valid = usize::try_from(current).ok().is_some_and(|c| {
            self.state
                .borrow()
                .panel
                .rows
                .get(c)
                .is_some_and(Option::is_some)
        });
        if !still_valid {
            app.set_stream_index(select.map_or(-1, |s| s as i32));
        }
        app.invoke_reveal_stream();
    }

    /// The subtitles in the preferred languages, in their order.
    fn chosen_subtitles(&self, languages: &[String]) -> Vec<String> {
        let state = self.state.borrow();
        let mut seen = HashSet::new();
        languages
            .iter()
            .flat_map(|lang| {
                state
                    .panel
                    .subtitles
                    .iter()
                    .filter(move |s| s.lang == *lang)
            })
            .filter(|s| seen.insert(s.url.clone()))
            .map(|s| s.url.clone())
            .collect()
    }

    /// Keeps what the addon called the stream, to select it next time.
    fn remember_stream(&self, video_id: &str, stream: &Stream) {
        let mut streams = self.paths.last_streams();
        streams.insert(
            video_id.to_owned(),
            LastStream {
                name: stream.name.clone().unwrap_or_default(),
                description: stream.details().to_owned(),
            },
        );
        if let Err(e) = self.paths.save_last_streams(&streams) {
            eprintln!("last streams: {e}");
        }
    }
}

/// Where `last` is among the answers: the same name and description.
fn find_stream(answers: &[Answer], last: &LastStream) -> Option<(usize, usize)> {
    answers.iter().enumerate().find_map(|(a, (_, answer))| {
        let streams = answer.as_ref()?.as_ref().ok()?;
        let i = streams.iter().position(|s| {
            s.url.is_some()
                && s.name.clone().unwrap_or_default() == last.name
                && s.details() == last.description
        })?;
        Some((a, i))
    })
}

/// For example `7 streams` or `7 streams, 2 not playable`.
fn count_text(playable: usize, hidden: usize) -> String {
    let streams = if playable == 1 {
        "1 stream".to_owned()
    } else {
        format!("{playable} streams")
    };
    if hidden == 0 {
        streams
    } else {
        format!("{streams}, {hidden} not playable")
    }
}

/// A target's window title, its code and label from an episode or a movie.
pub(super) fn label(code: Option<&str>, title: &str, year: &str) -> String {
    match (code, title.is_empty()) {
        (Some(code), false) => format!("{code} · {title}"),
        (Some(code), true) => code.to_owned(),
        (None, _) => year.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(name: &str, details: &str, url: bool) -> Stream {
        Stream {
            name: Some(name.into()),
            description: Some(details.into()),
            url: url.then(|| "https://x".to_owned()),
            ..Stream::default()
        }
    }

    #[test]
    fn the_last_stream_is_found_by_its_words() {
        let answers = vec![
            ("A".to_owned(), Some(Err("down".to_owned()))),
            (
                "B".to_owned(),
                Some(Ok(vec![
                    stream("[TB+] 2160p", "one", true),
                    stream("[TB+] 1080p", "two", true),
                    stream("[TB+] 1080p", "two", false),
                ])),
            ),
        ];
        let last = LastStream {
            name: "[TB+] 1080p".into(),
            description: "two".into(),
        };
        assert_eq!(find_stream(&answers, &last), Some((1, 1)));
        let gone = LastStream {
            name: "x".into(),
            description: "y".into(),
        };
        assert_eq!(find_stream(&answers, &gone), None);
    }

    #[test]
    fn counts_and_labels() {
        assert_eq!(count_text(1, 0), "1 stream");
        assert_eq!(count_text(7, 2), "7 streams, 2 not playable");
        assert_eq!(
            label(Some("S2 E4"), "The Pantry Job", "2024"),
            "S2 E4 · The Pantry Job"
        );
        assert_eq!(label(Some("S2 E4"), "", "2024"), "S2 E4");
        assert_eq!(label(None, "", "2025"), "2025");
    }
}
