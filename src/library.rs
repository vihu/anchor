//! The Stremio library: what the user watched and where they stopped, as
//! stremio-core keeps it (`src/types/library/library_item.rs`), and how
//! watching updates it (`src/models/player.rs`).
//!
//! Times in a library item are milliseconds. An item is written back to
//! the account whole, so its fields keep stremio-core's names and shapes
//! exactly, and its behavior hints keep keys anchor does not know.

use jiff::Timestamp;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::addon::{MetaPreview, Video};
use crate::watched::Watched;

/// Past this share of a video's length, the time spent in it marks it
/// watched.
const WATCHED_THRESHOLD: f64 = 0.7;
/// Past this share of a video's length, stopping counts as finishing it:
/// what is left are the credits.
pub const CREDITS_THRESHOLD: f64 = 0.9;
/// Continue watching shows at most this many titles.
const CONTINUE_WATCHING_MAX: usize = 100;
/// Removed items stay in sync for a year.
const SYNC_REMOVED_FOR: jiff::SignedDuration = jiff::SignedDuration::from_hours(365 * 24);
/// Below this offset, in milliseconds, a video starts from the beginning:
/// stremio-core sets the offset to 1 to point at the next episode.
const RESUME_FROM_MS: u64 = 5_000;

/// A title in the user's library, or one they watched without adding it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    /// The title's id, for example `tt0903747`.
    #[serde(rename = "_id")]
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its type, for example `series`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Its poster.
    #[serde(default, deserialize_with = "nonempty_text")]
    pub poster: Option<String>,
    /// `poster`, `landscape` or `square`.
    #[serde(default = "poster_shape", deserialize_with = "shape")]
    pub poster_shape: String,
    /// Not in the library: removed by the user, or never added.
    pub removed: bool,
    /// Watched without being added.
    pub temp: bool,
    /// When it was created.
    #[serde(default, rename = "_ctime", deserialize_with = "lenient_time")]
    pub ctime: Option<Timestamp>,
    /// When it last changed.
    #[serde(rename = "_mtime")]
    pub mtime: Timestamp,
    /// Where the user is in it.
    pub state: State,
    /// The title's hints, as its metadata gave them.
    #[serde(default)]
    pub behavior_hints: Map<String, Value>,
}

/// Where the user is in a title.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// When it was last watched.
    #[serde(default, deserialize_with = "lenient_time")]
    pub last_watched: Option<Timestamp>,
    /// Milliseconds watched of the current video.
    #[serde(default, deserialize_with = "millis")]
    pub time_watched: u64,
    /// Milliseconds into the current video: where it resumes.
    #[serde(default, deserialize_with = "millis")]
    pub time_offset: u64,
    /// Milliseconds watched of the whole title.
    #[serde(default, deserialize_with = "millis")]
    pub overall_time_watched: u64,
    /// Times the title was watched to the end.
    #[serde(default, deserialize_with = "millis")]
    pub times_watched: u32,
    /// 1 once the current video counts as watched.
    #[serde(default, deserialize_with = "millis")]
    pub flagged_watched: u32,
    /// Milliseconds long, the current video.
    #[serde(default, deserialize_with = "millis")]
    pub duration: u64,
    /// The current video, for example `tt0903747:2:4`.
    #[serde(default, rename = "video_id")]
    pub video_id: Option<String>,
    /// Which episodes were watched: a [`Watched`] field.
    #[serde(default)]
    pub watched: Option<String>,
    /// No notifications of new episodes.
    #[serde(default)]
    pub no_notif: bool,
}

// Public API
impl LibraryItem {
    /// A new item for a title the user starts watching without adding it.
    pub fn new(meta: &MetaPreview, now: Timestamp) -> Self {
        let mut item = Self {
            id: meta.id.clone(),
            name: String::new(),
            kind: String::new(),
            poster: None,
            poster_shape: poster_shape(),
            removed: true,
            temp: true,
            ctime: Some(now),
            mtime: now,
            state: State {
                last_watched: Some(now),
                ..State::default()
            },
            behavior_hints: Map::new(),
        };
        item.refresh(meta);
        item
    }

    /// Takes the title's name, type, poster and hints from its metadata,
    /// keeping where the user is.
    pub fn refresh(&mut self, meta: &MetaPreview) {
        self.name.clone_from(&meta.name);
        self.kind.clone_from(&meta.kind);
        self.poster.clone_from(&meta.poster);
        let hints = &meta.behavior_hints;
        let mut map = Map::new();
        if hints.is_live {
            map.insert("isLive".into(), Value::Bool(true));
        }
        map.insert(
            "defaultVideoId".into(),
            hints.default_video_id.clone().into(),
        );
        map.insert(
            "featuredVideoId".into(),
            hints.featured_video_id.clone().into(),
        );
        map.insert(
            "hasScheduledVideos".into(),
            hints.has_scheduled_videos.into(),
        );
        self.behavior_hints = map;
    }

    /// A live channel rather than a title with a length.
    pub fn is_live(&self) -> bool {
        self.behavior_hints.get("isLive").and_then(Value::as_bool) == Some(true)
            || self.kind == "tv"
    }

    /// Whether it shows in Continue watching: started, not finished, and
    /// not removed by the user.
    pub fn in_continue_watching(&self) -> bool {
        self.kind != "other"
            && (!self.removed || self.temp)
            && self.state.time_offset > 0
            && !self.is_live()
    }

    /// How far into the current video, from 0 to 1.
    pub fn progress(&self) -> f64 {
        if self.state.time_offset > 0 && self.state.duration > 0 {
            (self.state.time_offset as f64 / self.state.duration as f64).min(1.0)
        } else {
            0.0
        }
    }

    /// Whether it was watched to the end at least once.
    pub fn watched(&self) -> bool {
        self.state.times_watched > 0
    }

    /// Whether the account should keep it: not the `other` type, and in
    /// the library or removed within the last year.
    pub fn should_sync(&self, now: Timestamp) -> bool {
        let recently_removed = self.removed && self.mtime > now - SYNC_REMOVED_FOR;
        self.kind != "other" && (!self.removed || recently_removed)
    }

    /// Where `video_id` resumes, in seconds; `None` to start at the
    /// beginning.
    pub fn resume_at(&self, video_id: &str) -> Option<f64> {
        (self.state.video_id.as_deref() == Some(video_id)
            && self.state.time_offset >= RESUME_FROM_MS)
            .then(|| self.state.time_offset as f64 / 1000.0)
    }

    /// Which of `video_ids` (in [`bitfield order`]) were watched.
    ///
    /// [`bitfield order`]: crate::addon::MetaItem::bitfield_ids
    pub fn watched_videos(&self, video_ids: Vec<String>) -> Watched {
        match &self.state.watched {
            Some(field) => Watched::parse(field, video_ids.clone())
                .unwrap_or_else(|_| Watched::none(video_ids)),
            None => Watched::none(video_ids),
        }
    }

    /// mpv's first report of a playback, `time` milliseconds into
    /// `video_id`: the item points at that video from where it plays,
    /// whatever it pointed at before (another episode, or this one further
    /// on before a restart from the beginning).
    pub fn started(
        &mut self,
        video_id: &str,
        time: u64,
        duration: u64,
        video_ids: &[String],
        now: Timestamp,
    ) {
        self.time_changed(video_id, time, duration, video_ids, now);
        if self.state.time_offset != time {
            self.seek(time, duration, now);
        }
    }

    /// mpv reports `time` milliseconds into `video_id`, `duration` long:
    /// counts the time watched and, past 70 %, marks the video watched.
    ///
    /// `video_ids` are the title's videos in bitfield order, for the
    /// episode's watched mark.
    pub fn time_changed(
        &mut self,
        video_id: &str,
        time: u64,
        duration: u64,
        video_ids: &[String],
        now: Timestamp,
    ) {
        let video_id = if self.is_live() {
            self.id.clone()
        } else {
            video_id.to_owned()
        };
        let state = &mut self.state;
        state.last_watched = Some(now);
        if state.video_id.as_deref() == Some(video_id.as_str()) {
            let watched = time.saturating_sub(state.time_offset);
            state.time_watched = state.time_watched.saturating_add(watched);
            state.overall_time_watched = state.overall_time_watched.saturating_add(watched);
        } else {
            state.video_id = Some(video_id.clone());
            state.overall_time_watched = state
                .overall_time_watched
                .saturating_add(state.time_watched);
            state.time_watched = 0;
            state.flagged_watched = 0;
        }
        // A forward seek would count as watched time: the caller reports
        // seeks through `seek` instead.
        if time > state.time_offset {
            state.time_offset = time;
            state.duration = duration;
        }
        if !self.is_live()
            && self.state.flagged_watched == 0
            && self.state.duration > 0
            && self.state.time_watched as f64 > self.state.duration as f64 * WATCHED_THRESHOLD
        {
            self.state.flagged_watched = 1;
            self.state.times_watched = self.state.times_watched.saturating_add(1);
            // Without the title's videos the field cannot be read: left as
            // it is, as stremio-core leaves it.
            if !video_ids.is_empty() {
                let mut watched = self.watched_videos(video_ids.to_vec());
                watched.set(&video_id, true);
                self.state.watched = Some(watched.serialize());
            }
        }
        if self.temp && self.state.times_watched == 0 {
            self.removed = true;
        }
        if self.removed {
            self.temp = true;
        }
        self.mtime = now;
    }

    /// The user jumped to `time` milliseconds: it becomes the resume point,
    /// and the jump is not counted as watched.
    pub fn seek(&mut self, time: u64, duration: u64, now: Timestamp) {
        self.state.last_watched = Some(now);
        self.state.time_offset = time;
        self.state.duration = duration;
        self.mtime = now;
    }

    /// Playback stopped. Finished (watched, or into the credits), the title
    /// moves on to `next` (the next episode), or back to the start.
    pub fn stopped(&mut self, next: Option<&str>, now: Timestamp) {
        if self.is_live() {
            self.state.time_offset = 0;
            self.state.video_id = Some(self.id.clone());
        } else if self.state.flagged_watched == 1
            || self.state.time_offset as f64 > self.state.duration as f64 * CREDITS_THRESHOLD
        {
            self.state.time_offset = 0;
            if let Some(next) = next {
                self.advance_to(next);
            }
        }
        self.mtime = now;
    }

    /// Marks the whole title watched, or not.
    pub fn mark_watched(&mut self, watched: bool, now: Timestamp) {
        if watched {
            self.state.times_watched = self.state.times_watched.saturating_add(1);
            self.state.last_watched = Some(now);
            self.state.time_offset = 0;
        } else {
            self.state.times_watched = 0;
        }
        self.mtime = now;
    }

    /// Marks `video` (one of `video_ids`, in bitfield order) watched, or
    /// not.
    pub fn mark_video_watched(
        &mut self,
        video: &Video,
        watched: bool,
        video_ids: &[String],
        now: Timestamp,
    ) {
        if video_ids.is_empty() {
            return;
        }
        let mut field = self.watched_videos(video_ids.to_vec());
        field.set(&video.id, watched);
        self.state.watched = Some(field.serialize());
        if watched {
            let released = video
                .released
                .as_deref()
                .and_then(|r| r.parse::<Timestamp>().ok());
            self.state.last_watched = match (self.state.last_watched, released) {
                (Some(last), Some(released)) if last < released => Some(released),
                (None, released) => released,
                (last, _) => last,
            };
        }
        self.mtime = now;
    }
}

/// The time now as Stremio's apps write it: to the millisecond.
pub fn now() -> Timestamp {
    Timestamp::now()
        .round(jiff::Unit::Millisecond)
        .expect("rounding the present to the millisecond stays in range")
}

/// The account's library merged with the copy here: by title, the newer
/// item wins, so a change made while a sync was on its way survives it.
pub fn merge(local: Vec<LibraryItem>, remote: Vec<LibraryItem>) -> Vec<LibraryItem> {
    let mut merged = remote;
    for item in local {
        match merged.iter_mut().find(|i| i.id == item.id) {
            Some(slot) if item.mtime > slot.mtime => *slot = item,
            Some(_) => {}
            None => merged.push(item),
        }
    }
    merged
}

/// The titles in Continue watching: latest first, at most 100.
pub fn continue_watching(items: &[LibraryItem]) -> Vec<&LibraryItem> {
    let mut items: Vec<&LibraryItem> = items.iter().filter(|i| i.in_continue_watching()).collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.mtime));
    items.truncate(CONTINUE_WATCHING_MAX);
    items
}

// Private API
impl LibraryItem {
    /// Points the title at video `id`, from its start.
    fn advance_to(&mut self, id: &str) {
        let state = &mut self.state;
        state.video_id = Some(id.to_owned());
        state.overall_time_watched = state
            .overall_time_watched
            .saturating_add(state.time_watched);
        state.time_watched = 0;
        state.flagged_watched = 0;
        state.time_offset = 1;
    }
}

fn poster_shape() -> String {
    "poster".to_owned()
}

/// `poster`, `landscape` or `square`; anything else is `poster`.
fn shape<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(s) if s == "landscape" || s == "square" => s,
        _ => poster_shape(),
    })
}

/// Text where an empty string, `null` or anything else means none.
fn nonempty_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(s) if !s.is_empty() => Some(s),
        _ => None,
    })
}

/// A timestamp where `null`, an empty string or a broken one means none.
fn lenient_time<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Timestamp>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

/// A count or milliseconds, written by older apps as a fraction or `null`.
fn millis<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<u64> + Default,
{
    let value = match Value::deserialize(deserializer)? {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f.round() as u64)),
        _ => None,
    };
    Ok(value.and_then(|v| T::try_from(v).ok()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000;

    fn now() -> Timestamp {
        "2026-10-04T20:00:00Z".parse().unwrap()
    }

    fn meta() -> MetaPreview {
        serde_json::from_value(serde_json::json!({
            "id": "tt0903747", "type": "series", "name": "Breaking Bad",
            "poster": "https://images.example/bb.jpg",
            "behaviorHints": {"defaultVideoId": null, "hasScheduledVideos": true}
        }))
        .unwrap()
    }

    fn episodes() -> Vec<String> {
        (1..=7).map(|e| format!("tt0903747:1:{e}")).collect()
    }

    /// An item from the account, as the API sends it.
    const FROM_ACCOUNT: &str = r#"{
        "_id": "tt1375666", "name": "Inception", "type": "movie",
        "poster": "", "posterShape": "weird", "removed": false, "temp": false,
        "_ctime": null, "_mtime": "2026-09-01T10:00:00.123Z",
        "state": {"lastWatched": "2026-09-01T10:00:00Z", "timeWatched": 1200.4,
            "timeOffset": 3600000, "overallTimeWatched": 3600000, "timesWatched": 0,
            "flaggedWatched": 0, "duration": 8880000, "video_id": "tt1375666",
            "watched": null, "noNotif": false},
        "behaviorHints": {"defaultVideoId": "tt1375666", "featuredVideoId": null,
            "hasScheduledVideos": false, "somethingNew": 1}
    }"#;

    #[test]
    fn an_account_item_reads_leniently_and_keeps_unknown_hints() {
        let item: LibraryItem = serde_json::from_str(FROM_ACCOUNT).unwrap();
        assert_eq!(item.poster, None, "an empty poster is none");
        assert_eq!(item.poster_shape, "poster", "an unknown shape is poster");
        assert_eq!(item.ctime, None);
        assert_eq!(item.state.time_watched, 1200, "a fraction rounds");
        assert_eq!(item.resume_at("tt1375666"), Some(3600.0));
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["behaviorHints"]["somethingNew"], 1);
        assert_eq!(json["state"]["video_id"], "tt1375666");
        assert_eq!(json["_mtime"], "2026-09-01T10:00:00.123Z");
        assert!(json["poster"].is_null());
        assert!(json.get("_ctime").is_some());
    }

    #[test]
    fn a_new_item_is_temporary_until_added() {
        let item = LibraryItem::new(&meta(), now());
        assert!(item.removed && item.temp);
        assert_eq!(item.name, "Breaking Bad");
        assert_eq!(item.state.last_watched, Some(now()));
        assert_eq!(item.behavior_hints["hasScheduledVideos"], true);
        assert!(item.behavior_hints["defaultVideoId"].is_null());
        assert!(!item.in_continue_watching(), "not started");
    }

    #[test]
    fn watching_counts_time_and_marks_the_episode_past_70_percent() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        let episode = "tt0903747:1:2";
        let duration = 48 * MINUTE;
        for minute in 0..=34 {
            item.time_changed(episode, minute * MINUTE, duration, &ids, now());
        }
        assert_eq!(item.state.video_id.as_deref(), Some(episode));
        assert_eq!(item.state.time_offset, 34 * MINUTE);
        assert_eq!(item.state.time_watched, 34 * MINUTE);
        assert!(item.in_continue_watching());
        assert!(item.removed && item.temp, "watched, never added");
        assert_eq!(
            item.state.flagged_watched, 1,
            "34 of 48 minutes is past 70 %"
        );
        assert_eq!(item.state.times_watched, 1);
        assert!(item.watched_videos(ids.clone()).get(episode));
        assert!(!item.watched_videos(ids).get("tt0903747:1:1"));
    }

    #[test]
    fn a_new_episode_starts_from_where_it_plays() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        for minute in [0, 30] {
            item.time_changed("tt0903747:1:1", minute * MINUTE, 45 * MINUTE, &ids, now());
        }
        item.stopped(Some("tt0903747:1:2"), now());
        // Episode 1 stopped at 30 of 45; the next one is played from its
        // start instead.
        let mut other = item.clone();
        other.started("tt0903747:1:3", 0, 47 * MINUTE, &ids, now());
        for minute in 1..=10 {
            other.time_changed("tt0903747:1:3", minute * MINUTE, 47 * MINUTE, &ids, now());
        }
        assert_eq!(other.state.video_id.as_deref(), Some("tt0903747:1:3"));
        assert_eq!(other.state.time_offset, 10 * MINUTE);
        assert_eq!(other.state.duration, 47 * MINUTE);
        assert_eq!(other.state.time_watched, 10 * MINUTE);
    }

    #[test]
    fn a_restart_moves_the_resume_point_back() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        for minute in [0, 30] {
            item.time_changed("tt0903747:1:1", minute * MINUTE, 45 * MINUTE, &ids, now());
        }
        item.started("tt0903747:1:1", 0, 45 * MINUTE, &ids, now());
        assert_eq!(item.resume_at("tt0903747:1:1"), None, "from the beginning");
        item.time_changed("tt0903747:1:1", 5 * MINUTE, 45 * MINUTE, &ids, now());
        assert_eq!(item.state.time_offset, 5 * MINUTE);
    }

    #[test]
    fn without_the_videos_the_watched_field_stays() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        let mut watched = Watched::none(ids.clone());
        watched.set("tt0903747:1:1", true);
        watched.set("tt0903747:1:2", true);
        let field = watched.serialize();
        item.state.watched = Some(field.clone());
        for minute in 0..=40 {
            item.time_changed("tt0903747:1:3", minute * MINUTE, 45 * MINUTE, &[], now());
        }
        assert_eq!(item.state.flagged_watched, 1);
        assert_eq!(item.state.watched.as_deref(), Some(field.as_str()));
        let video = Video {
            id: "tt0903747:1:4".into(),
            ..Video::default()
        };
        item.mark_video_watched(&video, true, &[], now());
        assert_eq!(item.state.watched.as_deref(), Some(field.as_str()));
    }

    #[test]
    fn a_seek_is_not_watched_time() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        item.time_changed("tt0903747:1:1", MINUTE, 48 * MINUTE, &ids, now());
        item.seek(40 * MINUTE, 48 * MINUTE, now());
        item.time_changed("tt0903747:1:1", 41 * MINUTE, 48 * MINUTE, &ids, now());
        // The first report only starts the count; then one minute played.
        assert_eq!(item.state.time_watched, MINUTE);
        assert_eq!(item.state.flagged_watched, 0);
        assert_eq!(item.state.time_offset, 41 * MINUTE);
    }

    #[test]
    fn another_episode_starts_its_own_count() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        item.time_changed("tt0903747:1:1", 0, 48 * MINUTE, &ids, now());
        item.time_changed("tt0903747:1:1", 10 * MINUTE, 48 * MINUTE, &ids, now());
        item.time_changed("tt0903747:1:2", 0, 47 * MINUTE, &ids, now());
        assert_eq!(item.state.video_id.as_deref(), Some("tt0903747:1:2"));
        assert_eq!(item.state.time_watched, 0);
        // As stremio-core counts it: the episode's time again on leaving it.
        assert_eq!(item.state.overall_time_watched, 20 * MINUTE);
        assert_eq!(item.resume_at("tt0903747:1:1"), None, "the title moved on");
    }

    #[test]
    fn stopping_in_the_credits_moves_to_the_next_episode() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        for minute in 0..=45 {
            item.time_changed("tt0903747:1:3", minute * MINUTE, 48 * MINUTE, &ids, now());
        }
        item.stopped(Some("tt0903747:1:4"), now());
        assert_eq!(item.state.video_id.as_deref(), Some("tt0903747:1:4"));
        assert_eq!(item.state.time_offset, 1, "points at the next episode");
        assert!(item.in_continue_watching());
        assert_eq!(item.resume_at("tt0903747:1:4"), None, "from its start");

        // Stopped early: it resumes where it was.
        let mut early = LibraryItem::new(&meta(), now());
        early.time_changed("tt0903747:1:3", 10 * MINUTE, 48 * MINUTE, &ids, now());
        early.stopped(Some("tt0903747:1:4"), now());
        assert_eq!(early.resume_at("tt0903747:1:3"), Some(600.0));
    }

    #[test]
    fn mark_watched_clears_the_resume_point() {
        let mut item: LibraryItem = serde_json::from_str(FROM_ACCOUNT).unwrap();
        item.mark_watched(true, now());
        assert!(item.watched());
        assert!(!item.in_continue_watching());
        assert_eq!(item.mtime, now());
        item.mark_watched(false, now());
        assert!(!item.watched());
    }

    #[test]
    fn mark_one_episode_watched() {
        let mut item = LibraryItem::new(&meta(), now());
        let ids = episodes();
        let video = Video {
            id: "tt0903747:1:5".into(),
            released: Some("2008-02-24T05:00:00.000Z".into()),
            ..Video::default()
        };
        item.mark_video_watched(&video, true, &ids, now());
        assert!(item.watched_videos(ids.clone()).get("tt0903747:1:5"));
        item.mark_video_watched(&video, false, &ids, now());
        assert!(!item.watched_videos(ids).get("tt0903747:1:5"));
    }

    #[test]
    fn continue_watching_is_latest_first_without_removed_or_finished() {
        let base: LibraryItem = serde_json::from_str(FROM_ACCOUNT).unwrap();
        let item = |id: &str, mtime: &str, offset: u64, removed: bool, temp: bool| LibraryItem {
            id: id.into(),
            mtime: mtime.parse().unwrap(),
            removed,
            temp,
            state: State {
                time_offset: offset,
                ..base.state.clone()
            },
            ..base.clone()
        };
        let items = [
            item("old", "2026-01-01T00:00:00Z", 10, false, false),
            item("new", "2026-09-01T00:00:00Z", 10, true, true),
            item("removed", "2026-09-02T00:00:00Z", 10, true, false),
            item("finished", "2026-09-03T00:00:00Z", 0, false, false),
        ];
        let ids: Vec<&str> = continue_watching(&items)
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["new", "old"]);
    }

    #[test]
    fn now_is_to_the_millisecond() {
        let json = serde_json::to_string(&now()).unwrap();
        let fraction = json.trim_matches('"').split('.').nth(1).unwrap_or("Z");
        assert!(fraction.len() <= 4, "{json}");
    }

    #[test]
    fn merging_keeps_the_newer_item() {
        let base: LibraryItem = serde_json::from_str(FROM_ACCOUNT).unwrap();
        let at = |id: &str, mtime: &str| LibraryItem {
            id: id.into(),
            mtime: mtime.parse().unwrap(),
            ..base.clone()
        };
        let local = vec![
            at("a", "2026-10-04T12:00:00Z"),
            at("b", "2026-10-01T00:00:00Z"),
            at("here", "2026-10-04T00:00:00Z"),
        ];
        let remote = vec![
            at("a", "2026-10-02T00:00:00Z"),
            at("b", "2026-10-03T00:00:00Z"),
        ];
        let merged = merge(local, remote);
        let times: Vec<(String, String)> = merged
            .iter()
            .map(|i| (i.id.clone(), i.mtime.to_string()))
            .collect();
        assert_eq!(
            times,
            [
                ("a".into(), "2026-10-04T12:00:00Z".into()),
                ("b".into(), "2026-10-03T00:00:00Z".into()),
                ("here".into(), "2026-10-04T00:00:00Z".into()),
            ]
        );
    }

    #[test]
    fn sync_keeps_removed_items_for_a_year() {
        let mut item: LibraryItem = serde_json::from_str(FROM_ACCOUNT).unwrap();
        assert!(item.should_sync(now()));
        item.removed = true;
        assert!(item.should_sync(now()));
        assert!(!item.should_sync("2028-01-01T00:00:00Z".parse().unwrap()));
        item.kind = "other".into();
        item.removed = false;
        assert!(!item.should_sync(now()));
    }
}
