//! What mpv is playing: its position, the library item it moves (with
//! Stremio's rules, seeks apart), the now-playing pill and bar, and the
//! end: progress written back, the next episode pointed at, or the panel
//! again when mpv could not play the stream.

use std::rc::Rc;
use std::time::{Duration, Instant};

use anchor::addon::{MetaItem, MetaPreview};
use anchor::library::{self, LibraryItem};
use anchor::player::{Event, Outcome, Playback, Progress};
use jiff::Timestamp;
use slint::{ComponentHandle, SharedString, TimerMode};

use super::streams::Target;
use super::{Session, spawn, with_session};
use crate::art::Size;
use crate::text;
use crate::ui::NowPlaying;

/// How often progress is written to the account while mpv plays.
const PUSH_EVERY: Duration = Duration::from_secs(30);
/// Beyond the time that passed, a jump forward this large is a seek.
const SEEK_SLACK: f64 = 3.0;
/// How long a note stays at the foot of the window.
const TOAST_TIME: Duration = Duration::from_secs(5);

/// A title playing in mpv.
pub(super) struct Playing {
    token: u64,
    playback: Playback,
    target: Target,
    /// The last position mpv reported, and when.
    last: Progress,
    last_at: Instant,
    pushed_at: Instant,
    /// mpv reported a length: the stream plays.
    started: bool,
    /// When the library item last changed before this playback, to tell
    /// whether the account's copy is newer.
    base: Option<Timestamp>,
}

impl Playing {
    /// The playback `token` names, started on `target`.
    pub(super) fn new(token: u64, playback: Playback, target: Target) -> Self {
        Self {
            token,
            playback,
            target,
            last: Progress::default(),
            last_at: Instant::now(),
            pushed_at: Instant::now(),
            started: false,
            base: None,
        }
    }

    /// Asks mpv to quit.
    pub(super) fn stop(&self) {
        self.playback.stop();
    }

    /// The video playing.
    pub(super) fn video_id(&self) -> &str {
        &self.target.video_id
    }
}

impl Session {
    /// A token for the next playback, so events of an earlier one are told
    /// apart.
    pub(super) fn next_token(&self) -> u64 {
        let mut state = self.state.borrow_mut();
        state.tokens += 1;
        state.tokens
    }

    /// Another stream replaces `previous`: where it got to is saved, and
    /// its mpv is asked to quit.
    pub(super) fn replace_playing(&self, previous: Playing) {
        self.wrap_up(&previous, previous.last);
        previous.stop();
    }

    /// mpv started: the pill, the bar, and the page say so; the account's
    /// copy of the title is fetched, in case another device moved it on.
    pub(super) fn start_playing(self: &Rc<Self>, mut playing: Playing) {
        let target = playing.target.clone();
        playing.base = self
            .state
            .borrow()
            .library
            .iter()
            .find(|i| i.id == target.meta_id)
            .map(|i| i.mtime);
        if let Some((key, _)) = self.key() {
            let (api, token, id) = (self.api.clone(), playing.token, target.meta_id.clone());
            spawn(
                move || api.library_items(&key, &[id.as_str()]).unwrap_or_default(),
                move |s, items| s.fresh_item(token, items),
            );
        }
        let label = target.code.clone().unwrap_or_else(|| target.name.clone());
        self.state.borrow_mut().playing = Some(playing);
        if let Some(app) = self.app.upgrade() {
            let now = app.global::<NowPlaying>();
            now.set_active(true);
            now.set_label(label.into());
            now.set_title(
                match &target.code {
                    Some(_) => {
                        format!("{} · {}", target.name, target.label.replacen(" · ", " ", 1))
                    }
                    None => target.name.clone(),
                }
                .into(),
            );
            now.set_stream(SharedString::new());
            now.set_time("Starting…".into());
            now.set_progress(0.0);
            now.set_has_still(false);
        }
        if let Some(url) = &target.picture {
            self.art.borrow_mut().request(url, Size::Still);
        }
        if let Some(stream) = self.paths.last_streams().get(&target.video_id)
            && let Some(app) = self.app.upgrade()
        {
            let name = stream.name.replace('\n', " ");
            app.global::<NowPlaying>().set_stream(name.into());
        }
        self.show_playing();
    }

    /// The pill: the playing title's page.
    pub(super) fn open_playing(self: &Rc<Self>) {
        let target = self
            .state
            .borrow()
            .playing
            .as_ref()
            .map(|p| p.target.clone());
        if let Some(target) = target {
            self.open_title(MetaPreview {
                id: target.meta_id,
                kind: target.kind,
                name: target.name,
                ..MetaPreview::default()
            });
        }
    }

    /// The bar's Stop: quits mpv.
    pub(super) fn stop_playing(&self) {
        if let Some(playing) = self.state.borrow().playing.as_ref() {
            playing.stop();
        }
    }

    /// Something happened to the mpv of playback `token`.
    pub(super) fn player_event(self: &Rc<Self>, token: u64, event: Event) {
        if self.state.borrow().playing.as_ref().map(|p| p.token) != Some(token) {
            return;
        }
        match event {
            Event::Progress(progress) => self.progress(progress),
            Event::Ended(last, outcome) => self.ended(last, outcome),
        }
    }

    /// A picture for the now-playing bar arrived.
    pub(super) fn playing_art_ready(&self, url: &str, image: &slint::Image) {
        let wanted = self
            .state
            .borrow()
            .playing
            .as_ref()
            .and_then(|p| p.target.picture.clone());
        if wanted.as_deref() == Some(url)
            && let Some(app) = self.app.upgrade()
        {
            let now = app.global::<NowPlaying>();
            now.set_still(image.clone());
            now.set_has_still(true);
        }
    }

    /// The window closes: the position mpv reached is written to the
    /// account before anchor exits. mpv itself keeps playing.
    pub(super) fn finish_playing(&self) {
        let Some(playing) = self.state.borrow_mut().playing.take() else {
            return;
        };
        if !playing.started {
            return;
        }
        if let Some(item) = self.apply_progress(&playing, playing.last) {
            self.keep_item(item, true);
        }
    }

    /// Shows a note at the foot of the window for a few seconds.
    pub(super) fn toast(&self, text: &str) {
        if let Some(app) = self.app.upgrade() {
            app.set_toast(text.into());
        }
        self.toast_timer
            .start(TimerMode::SingleShot, TOAST_TIME, || {
                with_session(|s| {
                    if let Some(app) = s.app.upgrade() {
                        app.set_toast(SharedString::new());
                    }
                });
            });
    }
}

// Private
impl Session {
    fn progress(self: &Rc<Self>, progress: Progress) {
        let push = {
            let mut state = self.state.borrow_mut();
            let Some(playing) = state.playing.as_mut() else {
                return;
            };
            let paused_changed = playing.last.paused != progress.paused;
            playing.started |= progress.duration > 0.0;
            let push =
                playing.started && (playing.pushed_at.elapsed() >= PUSH_EVERY || paused_changed);
            if push {
                playing.pushed_at = Instant::now();
            }
            push
        };
        let item = {
            let state = self.state.borrow();
            let playing = state.playing.as_ref().expect("checked above");
            if playing.started {
                self.apply_progress(playing, progress)
            } else {
                None
            }
        };
        {
            let mut state = self.state.borrow_mut();
            if let Some(playing) = state.playing.as_mut() {
                playing.last = progress;
                playing.last_at = Instant::now();
            }
        }
        if let Some(item) = item {
            self.keep_item(item, push);
        }
        if let Some(app) = self.app.upgrade() {
            let now = app.global::<NowPlaying>();
            let label = self
                .state
                .borrow()
                .playing
                .as_ref()
                .map(|p| {
                    p.target
                        .code
                        .clone()
                        .unwrap_or_else(|| p.target.name.clone())
                })
                .unwrap_or_default();
            now.set_label(format!("{label} · {}", text::clock(progress.position)).into());
            if progress.duration > 0.0 {
                now.set_time(
                    format!(
                        "{} / {}{}",
                        text::clock(progress.position),
                        text::clock(progress.duration),
                        if progress.paused { " · paused" } else { "" }
                    )
                    .into(),
                );
                now.set_progress((progress.position / progress.duration).clamp(0.0, 1.0) as f32);
            }
        }
    }

    fn ended(self: &Rc<Self>, last: Progress, outcome: Outcome) {
        let Some(playing) = self.state.borrow_mut().playing.take() else {
            return;
        };
        if let Some(app) = self.app.upgrade() {
            app.global::<NowPlaying>().set_active(false);
        }
        let target = playing.target.clone();
        if let Outcome::Failed(why) = &outcome
            && !playing.started
        {
            self.show_playing();
            self.stream_failed(target, why);
            return;
        }
        if playing.started {
            // mpv also says the file ended when a stream drops: only the
            // credits count as finishing it.
            let finished = outcome == Outcome::Finished
                && last.position >= last.duration * library::CREDITS_THRESHOLD;
            let last = if finished {
                Progress {
                    position: last.duration,
                    ..last
                }
            } else {
                last
            };
            self.wrap_up(&playing, last);
            let what = target.code.clone().unwrap_or_else(|| target.name.clone());
            self.toast(&if finished {
                format!("Finished {what} · saved to your Stremio account")
            } else {
                format!(
                    "Saved to your Stremio account · {what} at {}",
                    text::clock(last.position)
                )
            });
        }
        self.show_playing();
        self.refresh_home();
    }

    /// Saves where `playing` stopped, at `last`: the library item moves on as
    /// Stremio's rules say (the next episode when this one is done), and
    /// goes to the account.
    fn wrap_up(&self, playing: &Playing, last: Progress) {
        if !playing.started {
            return;
        }
        if let Some(mut item) = self.apply_progress(playing, last) {
            let next = self.next_video(&playing.target);
            item.stopped(next.as_deref(), library::now());
            self.keep_item(item, true);
        }
    }

    /// The account's copy of the title playing arrived: when another device
    /// moved it on since the last sync, playing carries on from that.
    fn fresh_item(&self, token: u64, items: Vec<LibraryItem>) {
        let Some(mut remote) = items.into_iter().next() else {
            return;
        };
        let item = {
            let state = self.state.borrow();
            let Some(playing) = state.playing.as_ref().filter(|p| p.token == token) else {
                return;
            };
            // Newer than the copy this playback started from: the account
            // moved on elsewhere. Playing carries on from its copy, at the
            // position mpv is at.
            if playing.base.is_some_and(|base| remote.mtime <= base) {
                return;
            }
            if playing.last.duration > 0.0 {
                let ids = state
                    .metas
                    .get(&playing.target.meta_id)
                    .map(MetaItem::bitfield_ids)
                    .unwrap_or_default();
                remote.started(
                    &playing.target.video_id,
                    millis(playing.last.position),
                    millis(playing.last.duration),
                    &ids,
                    library::now(),
                );
            }
            remote
        };
        self.keep_item(item, false);
    }

    /// The library item after `progress`: a seek moves the resume point, a
    /// stretch of playing counts as watched; `None` before mpv reports a
    /// length.
    fn apply_progress(&self, playing: &Playing, progress: Progress) -> Option<LibraryItem> {
        if progress.duration <= 0.0 {
            return None;
        }
        let state = self.state.borrow();
        let now = library::now();
        let target = &playing.target;
        let meta = state.metas.get(&target.meta_id);
        let mut item = state
            .library
            .iter()
            .find(|i| i.id == target.meta_id)
            .cloned()
            .unwrap_or_else(|| {
                let preview = meta.map_or_else(
                    || MetaPreview {
                        id: target.meta_id.clone(),
                        kind: target.kind.clone(),
                        name: target.name.clone(),
                        ..MetaPreview::default()
                    },
                    |m| m.preview.clone(),
                );
                LibraryItem::new(&preview, now)
            });
        let (time, duration) = (millis(progress.position), millis(progress.duration));
        // Without the metadata the episodes' order is unknown: the watched
        // marks are then left alone.
        let ids = meta.map(MetaItem::bitfield_ids).unwrap_or_default();
        if playing.last.duration <= 0.0 {
            // The first report: the item points at this video, from here.
            item.started(&target.video_id, time, duration, &ids, now);
        } else {
            // Paused, no time passes: any move is a seek.
            let allowed = if playing.last.paused {
                0.0
            } else {
                playing.last_at.elapsed().as_secs_f64()
            };
            let moved = progress.position - playing.last.position;
            if moved > allowed + SEEK_SLACK || moved < -1.0 {
                item.seek(time, duration, now);
            } else {
                item.time_changed(&target.video_id, time, duration, &ids, now);
            }
        }
        Some(item)
    }

    /// Keeps `item` in the library; `push` writes it to the account too.
    fn keep_item(&self, item: LibraryItem, push: bool) {
        if push {
            self.save_item(item);
        } else {
            let mut state = self.state.borrow_mut();
            match state.library.iter_mut().find(|i| i.id == item.id) {
                Some(slot) => *slot = item,
                None => state.library.push(item),
            }
        }
    }

    /// The episode after `target`'s, as Stremio picks it.
    fn next_video(&self, target: &Target) -> Option<String> {
        let state = self.state.borrow();
        let meta = state.metas.get(&target.meta_id)?;
        meta.next_video(&target.video_id, library::now())
            .map(|v| v.id.clone())
    }

    /// The title page and its episode list say whether mpv plays them.
    fn show_playing(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let playing = self
            .state
            .borrow()
            .playing
            .as_ref()
            .map(|p| p.target.meta_id.clone());
        let page = self.state.borrow().page.as_ref().map(|p| p.id().to_owned());
        let here = playing.is_some() && playing == page;
        let mut details = app.get_title_details();
        details.playing = here;
        app.set_title_details(details);
        app.set_title_playing(here);
        self.refresh_page();
        self.refresh_episodes();
    }
}

/// Seconds as Stremio's milliseconds.
fn millis(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}
