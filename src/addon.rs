//! The Stremio addon protocol: manifests, which addon answers what, the
//! resource URLs with their extras, and the catalogs, metadata, streams and
//! subtitles addons answer with.
//!
//! The types follow stremio-core's (`src/types/addon/`, `src/types/resource/`),
//! which is not on crates.io, but only as far as anchor uses them, and are
//! lenient: an item an addon gets wrong is skipped, not the whole answer.

use std::cmp::Ordering;

use jiff::Timestamp;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::net::{self, Client};

/// The end of an addon's transport URL, which a resource's path replaces.
const MANIFEST: &str = "/manifest.json";
/// What stays as it is in each part of a resource's path: JavaScript's
/// `encodeURIComponent`, as stremio-core encodes ids and extras.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// An installed addon: where it is and what it says it does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Addon {
    /// The URL of its manifest, ending in `/manifest.json`.
    pub transport_url: String,
    /// What it says it does.
    pub manifest: Manifest,
}

/// An addon's manifest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Its id, for example `com.linvo.cinemeta`.
    pub id: String,
    /// Its version.
    #[serde(default)]
    pub version: String,
    /// Its name.
    pub name: String,
    /// What it is, in a sentence.
    #[serde(default)]
    pub description: Option<String>,
    /// Its logo.
    #[serde(default)]
    pub logo: Option<String>,
    /// The content types it serves, for example `movie` and `series`.
    #[serde(default)]
    pub types: Vec<String>,
    /// The resources it serves.
    #[serde(default)]
    pub resources: Vec<Resource>,
    /// The id prefixes it serves; any id when absent.
    #[serde(default)]
    pub id_prefixes: Option<Vec<String>>,
    /// Its catalogs.
    #[serde(default)]
    pub catalogs: Vec<Catalog>,
}

/// A resource an addon serves: its name alone, or with the types and id
/// prefixes it serves it for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Resource {
    /// `"stream"`: for every type and prefix of the manifest.
    Short(String),
    /// `{"name": "stream", "types": [...], "idPrefixes": [...]}`.
    Full {
        /// The resource, for example `stream`.
        name: String,
        /// The types it serves it for; the manifest's when absent.
        #[serde(default)]
        types: Option<Vec<String>>,
        /// The id prefixes it serves it for; the manifest's when absent.
        #[serde(default, rename = "idPrefixes")]
        id_prefixes: Option<Vec<String>>,
    },
}

/// A catalog an addon offers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    /// The content type, for example `movie`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Its id within the addon.
    pub id: String,
    /// What to call it.
    #[serde(default)]
    pub name: Option<String>,
    /// The extras it takes (genre, search, skip).
    #[serde(default)]
    pub extra: Vec<ExtraProp>,
    /// The older way to name the extras it requires.
    #[serde(default)]
    pub extra_required: Vec<String>,
    /// The older way to name the extras it takes.
    #[serde(default)]
    pub extra_supported: Vec<String>,
}

/// An extra a catalog takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraProp {
    /// For example `genre`, `search` or `skip`.
    pub name: String,
    /// The catalog answers only with it.
    #[serde(default)]
    pub is_required: bool,
    /// The values it takes, for example the genres.
    #[serde(default)]
    pub options: Vec<String>,
}

/// A title in a catalog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPreview {
    /// Its id, for example `tt0903747`.
    pub id: String,
    /// Its type, for example `series`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Its name.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub name: String,
    /// The poster's URL.
    #[serde(default)]
    pub poster: Option<String>,
    /// A wide picture's URL.
    #[serde(default)]
    pub background: Option<String>,
    /// The title as a logo.
    #[serde(default)]
    pub logo: Option<String>,
    /// What it is about.
    #[serde(default)]
    pub description: Option<String>,
    /// For example `2010` or `2008-2013`.
    #[serde(default, deserialize_with = "text_or_number")]
    pub release_info: Option<String>,
    /// For example `148 min`.
    #[serde(default, deserialize_with = "text_or_number")]
    pub runtime: Option<String>,
    /// Its genres.
    #[serde(default)]
    pub genres: Vec<String>,
    /// For example `8.8`.
    #[serde(default, deserialize_with = "text_or_number")]
    pub imdb_rating: Option<String>,
    /// People and categories, for example the cast and directors.
    #[serde(default)]
    pub links: Vec<Link>,
    /// Hints, for example the video a movie plays.
    #[serde(default)]
    pub behavior_hints: MetaHints,
}

/// A link on a title: a cast member, a director, a genre, a rating.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// For example `Ana Dimas`.
    pub name: String,
    /// For example `Cast`, `Directors`, `Genres` or `imdb`.
    pub category: String,
}

/// Hints on a title.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaHints {
    /// The video a movie (or a title with one video) plays.
    #[serde(default)]
    pub default_video_id: Option<String>,
}

/// A title with everything about it: its videos too.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MetaItem {
    /// What the catalog shows.
    #[serde(flatten)]
    pub preview: MetaPreview,
    /// Its videos: a series' episodes; usually none for a movie.
    #[serde(default, deserialize_with = "lenient_list")]
    pub videos: Vec<Video>,
}

/// A video of a title, for example an episode.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Video {
    /// Its id, for example `tt0903747:1:1`.
    pub id: String,
    /// Its title (addons send it as `title` or `name`).
    #[serde(default)]
    pub title: Option<String>,
    /// Its title, the other way.
    #[serde(default)]
    pub name: Option<String>,
    /// When it came out, as ISO 8601.
    #[serde(default)]
    pub released: Option<String>,
    /// What happens in it (addons send it as `overview` or `description`).
    #[serde(default)]
    pub overview: Option<String>,
    /// What happens in it, the other way.
    #[serde(default)]
    pub description: Option<String>,
    /// A still from it.
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// Its season; 0 for specials.
    #[serde(default)]
    pub season: Option<u32>,
    /// Its number in the season.
    #[serde(default)]
    pub episode: Option<u32>,
}

/// A stream an addon offers for a video.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    /// A direct link: the only kind anchor plays.
    #[serde(default)]
    pub url: Option<String>,
    /// A torrent, which anchor lists but does not play.
    #[serde(default)]
    pub info_hash: Option<String>,
    /// A page to open in the browser instead.
    #[serde(default)]
    pub external_url: Option<String>,
    /// The short name, often the quality, for example `[TB+] 2160p`.
    #[serde(default)]
    pub name: Option<String>,
    /// The details, over several lines.
    #[serde(default)]
    pub description: Option<String>,
    /// The details, the older way.
    #[serde(default)]
    pub title: Option<String>,
    /// Hints, for example headers the link needs.
    #[serde(default)]
    pub behavior_hints: StreamHints,
}

/// Hints on a stream.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamHints {
    /// Streams of the same group, for the next episode.
    #[serde(default)]
    pub binge_group: Option<String>,
    /// The file's name.
    #[serde(default)]
    pub filename: Option<String>,
    /// The file's size in bytes.
    #[serde(default)]
    pub video_size: Option<u64>,
    /// Headers the link needs.
    #[serde(default)]
    pub proxy_headers: Option<ProxyHeaders>,
}

/// Headers a stream's link needs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyHeaders {
    /// Sent with the request, by name.
    #[serde(default)]
    pub request: serde_json::Map<String, Value>,
}

/// A subtitle file for a video.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subtitle {
    /// Its id at the addon.
    #[serde(default)]
    pub id: String,
    /// Where it is.
    pub url: String,
    /// Its language, for example `eng`.
    pub lang: String,
}

/// What can go wrong asking an addon.
pub type Error = net::Error;

// Public API
impl Addon {
    /// Whether anchor can ask it over HTTP: its transport URL's path ends in
    /// `/manifest.json` (not the legacy `/stremio/v1` kind).
    pub fn speaks_http(&self) -> bool {
        let path = self
            .transport_url
            .split(['?', '#'])
            .next()
            .unwrap_or_default();
        (path.starts_with("https://") || path.starts_with("http://")) && path.ends_with(MANIFEST)
    }

    /// Whether the addon serves `resource` (for example `stream`) for
    /// content of type `kind` with this `id`.
    pub fn serves(&self, resource: &str, kind: &str, id: &str) -> bool {
        // The short form takes the manifest's types and prefixes; the full
        // form its own, where no types means none and no prefixes any id.
        self.manifest.resources.iter().any(|r| {
            let (name, types, prefixes) = match r {
                Resource::Short(name) => (
                    name,
                    Some(&self.manifest.types),
                    self.manifest.id_prefixes.as_ref(),
                ),
                Resource::Full {
                    name,
                    types,
                    id_prefixes,
                } => (name, types.as_ref(), id_prefixes.as_ref()),
            };
            name == resource
                && types.is_some_and(|t| t.iter().any(|t| t == kind))
                && prefixes.is_none_or(|p| {
                    p.is_empty() || p.iter().any(|prefix| id.starts_with(prefix.as_str()))
                })
        })
    }

    /// The URL of `resource` for `kind` and `id`, with `extra` as name and
    /// value pairs.
    pub fn resource_url(
        &self,
        resource: &str,
        kind: &str,
        id: &str,
        extra: &[(&str, &str)],
    ) -> String {
        resource_url(&self.transport_url, resource, kind, id, extra)
    }
}

impl Catalog {
    /// What to call it: its name, else its id.
    pub fn title(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// Whether it takes the extra `name`.
    pub fn takes(&self, name: &str) -> bool {
        self.extra.iter().any(|e| e.name == name) || self.extra_supported.iter().any(|e| e == name)
    }

    /// The extras it cannot answer without.
    pub fn required(&self) -> Vec<&str> {
        self.extra
            .iter()
            .filter(|e| e.is_required)
            .map(|e| e.name.as_str())
            .chain(self.extra_required.iter().map(String::as_str))
            .collect()
    }

    /// Whether it lists without any extra: what Home and the sidebar show.
    pub fn browsable(&self) -> bool {
        self.required().is_empty()
    }

    /// Whether it can be searched.
    pub fn searchable(&self) -> bool {
        self.takes("search")
    }

    /// The genres it can filter by.
    pub fn genres(&self) -> &[String] {
        self.extra
            .iter()
            .find(|e| e.name == "genre")
            .map_or(&[], |e| e.options.as_slice())
    }
}

impl MetaPreview {
    /// The rating out of 10, from the field or from an IMDb link.
    pub fn rating(&self) -> Option<&str> {
        self.imdb_rating
            .as_deref()
            .or_else(|| {
                self.links
                    .iter()
                    .find(|l| l.category.eq_ignore_ascii_case("imdb"))
                    .map(|l| l.name.as_str())
            })
            .filter(|r| !r.is_empty())
    }

    /// Its genres: the list, else the links under `Genres`.
    pub fn genre_names(&self) -> Vec<&str> {
        if self.genres.is_empty() {
            self.people("Genres")
        } else {
            self.genres.iter().map(String::as_str).collect()
        }
    }

    /// The names linked under `category`, for example `Cast`.
    pub fn people(&self, category: &str) -> Vec<&str> {
        self.links
            .iter()
            .filter(|l| l.category.eq_ignore_ascii_case(category))
            .map(|l| l.name.as_str())
            .collect()
    }
}

impl MetaItem {
    /// The video a movie plays: its default one, else its only one, else
    /// one with the title's id.
    pub fn movie_video_id(&self) -> String {
        self.preview
            .behavior_hints
            .default_video_id
            .clone()
            .or_else(|| match self.videos.as_slice() {
                [only] => Some(only.id.clone()),
                _ => None,
            })
            .unwrap_or_else(|| self.preview.id.clone())
    }

    /// The videos as Stremio shows them, each id once: those with a season
    /// first, by season (specials, season 0, last) and episode; then the rest
    /// by release, oldest first when any has a season and newest first when
    /// none has, undated last.
    pub fn display_videos(&self) -> Vec<&Video> {
        let mut seen = std::collections::HashSet::new();
        let mut videos: Vec<&Video> = self.videos.iter().filter(|v| seen.insert(&v.id)).collect();
        let seasons = videos.iter().any(|v| v.season.is_some());
        videos.sort_by(|a, b| match (a.season, b.season) {
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(sa), Some(sb)) => {
                let last = |s: u32| if s == 0 { u32::MAX } else { s };
                last(sa).cmp(&last(sb)).then(a.episode.cmp(&b.episode))
            }
            (None, None) => match (a.released_ms(), b.released_ms()) {
                (Some(ra), Some(rb)) if seasons => ra.cmp(&rb),
                (Some(ra), Some(rb)) => rb.cmp(&ra),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        });
        videos
    }

    /// The video ids in the order Stremio's watched bitfield counts them:
    /// by season, episode and release, each missing one first.
    pub fn bitfield_ids(&self) -> Vec<String> {
        let mut videos: Vec<&Video> = self.videos.iter().collect();
        let key = |v: &Video| {
            (
                v.season.map_or(i64::MIN, i64::from),
                v.episode.map_or(i64::MIN, i64::from),
                v.released_ms().unwrap_or(i64::MIN),
            )
        };
        videos.sort_by_key(|v| key(v));
        videos.into_iter().map(|v| v.id.clone()).collect()
    }

    /// The video after `id` that Stremio plays next: the next in the list,
    /// unless it is a special after a regular episode or not out yet.
    pub fn next_video(&self, id: &str, now: Timestamp) -> Option<&Video> {
        let videos = self.display_videos();
        let at = videos.iter().position(|v| v.id == id)?;
        let (current, next) = (videos[at], *videos.get(at + 1)?);
        let out = next
            .released_ms()
            .is_none_or(|released| released <= now.as_millisecond());
        (next.season != Some(0) || current.season == next.season)
            .then_some(next)
            .filter(|_| out)
    }
}

impl Video {
    /// When it came out, in milliseconds since 1970.
    pub fn released_ms(&self) -> Option<i64> {
        self.released
            .as_deref()?
            .parse::<Timestamp>()
            .ok()
            .map(Timestamp::as_millisecond)
    }

    /// Its title, whichever field it came in.
    pub fn title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or_default()
    }

    /// What happens in it, whichever field it came in.
    pub fn overview(&self) -> &str {
        self.overview
            .as_deref()
            .or(self.description.as_deref())
            .unwrap_or_default()
    }
}

impl Stream {
    /// The details, whichever field they came in.
    pub fn details(&self) -> &str {
        self.description
            .as_deref()
            .or(self.title.as_deref())
            .unwrap_or_default()
    }

    /// The headers its link needs, as name and value.
    pub fn headers(&self) -> Vec<(String, String)> {
        self.behavior_hints
            .proxy_headers
            .as_ref()
            .map(|h| {
                h.request
                    .iter()
                    .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Asks addons for catalogs, metadata, streams and subtitles.
#[derive(Clone, Debug, Default)]
pub struct Addons {
    client: Client,
}

impl Addons {
    /// Asks through `client`.
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// A page of `catalog`, filtered by `extra` (for example a genre, a
    /// search or `skip` for the next page).
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the addon cannot be reached or answers
    /// with something that is not a catalog.
    pub fn catalog(
        &self,
        addon: &Addon,
        catalog: &Catalog,
        extra: &[(&str, &str)],
    ) -> Result<Vec<MetaPreview>, Error> {
        let url = addon.resource_url("catalog", &catalog.kind, &catalog.id, extra);
        let answer: Value = self.client.get(&url)?;
        Ok(lenient(answer.get("metas")))
    }

    /// The title `id` of type `kind`.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the addon cannot be reached or answers
    /// without a title.
    pub fn meta(&self, addon: &Addon, kind: &str, id: &str) -> Result<MetaItem, Error> {
        #[derive(Deserialize)]
        struct Answer {
            meta: MetaItem,
        }
        let url = addon.resource_url("meta", kind, id, &[]);
        Ok(self.client.get::<Answer>(&url)?.meta)
    }

    /// The streams for video `id` of type `kind`.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the addon cannot be reached or answers
    /// with something that is not a list of streams.
    pub fn streams(&self, addon: &Addon, kind: &str, id: &str) -> Result<Vec<Stream>, Error> {
        let url = addon.resource_url("stream", kind, id, &[]);
        let answer: Value = self.client.get(&url)?;
        Ok(lenient(answer.get("streams")))
    }

    /// The subtitles for video `id` of type `kind`; `extra` can name the
    /// file (`filename`, `videoSize`) for a closer match.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the addon cannot be reached or answers
    /// with something that is not a list of subtitles.
    pub fn subtitles(
        &self,
        addon: &Addon,
        kind: &str,
        id: &str,
        extra: &[(&str, &str)],
    ) -> Result<Vec<Subtitle>, Error> {
        let url = addon.resource_url("subtitles", kind, id, extra);
        let answer: Value = self.client.get(&url)?;
        Ok(lenient(answer.get("subtitles")))
    }
}

/// The transport URL with `/manifest.json` replaced by
/// `/{resource}/{kind}/{id}.json`, or `/{resource}/{kind}/{id}/{extra}.json`
/// with the extras as `name=value` joined by `&`, every part encoded as
/// stremio-core encodes it. A query on the transport URL stays.
pub fn resource_url(
    transport_url: &str,
    resource: &str,
    kind: &str,
    id: &str,
    extra: &[(&str, &str)],
) -> String {
    let part = |text| utf8_percent_encode(text, URI_COMPONENT);
    let (resource, kind, id) = (part(resource), part(kind), part(id));
    let path = if extra.is_empty() {
        format!("/{resource}/{kind}/{id}.json")
    } else {
        let extra: Vec<String> = extra
            .iter()
            .map(|(name, value)| format!("{}={}", part(name), part(value)))
            .collect();
        format!("/{resource}/{kind}/{id}/{}.json", extra.join("&"))
    };
    transport_url.replace(MANIFEST, &path)
}

/// A string that may come as `null`, read as empty.
fn null_as_empty<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// Text that some addons send as a number, for example a rating or a year.
fn text_or_number<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => Some(text),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

/// The items of `list` that parse; the rest are skipped.
fn lenient<T: serde::de::DeserializeOwned>(list: Option<&Value>) -> Vec<T> {
    list.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| T::deserialize(item).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn lenient_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    Ok(lenient(Some(&value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn cinemeta() -> Addon {
        Addon {
            transport_url: "https://v3-cinemeta.strem.io/manifest.json".into(),
            manifest: serde_json::from_str(&fixture("cinemeta-manifest.json")).unwrap(),
        }
    }

    #[test]
    fn cinemetas_manifest_reads() {
        let addon = cinemeta();
        assert_eq!(addon.manifest.id, "com.linvo.cinemeta");
        assert!(addon.speaks_http());
        let top = &addon.manifest.catalogs[0];
        assert_eq!(
            (top.kind.as_str(), top.id.as_str(), top.title()),
            ("movie", "top", "Popular")
        );
        assert!(top.browsable() && top.searchable());
        assert!(top.genres().contains(&"Action".to_owned()));
        assert!(addon.serves("meta", "series", "tt0903747"));
        assert!(!addon.serves("meta", "series", "kitsu:1"), "not its prefix");
        assert!(
            !addon.serves("stream", "movie", "tt1375666"),
            "not its resource"
        );
        assert!(!addon.serves("meta", "channel", "tt1"), "not its type");
    }

    #[test]
    fn resources_in_both_forms() {
        let addon = Addon {
            transport_url: "https://streams.example/eyJhIjoxfQ==/manifest.json".into(),
            manifest: serde_json::from_value(serde_json::json!({
                "id": "aio", "name": "AIOStreams", "types": ["movie", "series"],
                "resources": [
                    "subtitles",
                    {"name": "stream", "types": ["movie", "series", "anime"], "idPrefixes": ["tt", "kitsu"]}
                ]
            }))
            .unwrap(),
        };
        assert!(addon.serves("stream", "anime", "kitsu:1:2"));
        assert!(!addon.serves("stream", "movie", "tmdb:1"));
        assert!(
            addon.serves("subtitles", "movie", "anything"),
            "no prefixes: any id"
        );
        assert!(addon.speaks_http());
    }

    #[test]
    fn legacy_extras_count() {
        let catalog: Catalog = serde_json::from_value(serde_json::json!({
            "type": "movie", "id": "search", "name": "Search",
            "extraRequired": ["search"], "extraSupported": ["search"]
        }))
        .unwrap();
        assert!(catalog.searchable());
        assert!(!catalog.browsable());
        assert_eq!(catalog.required(), ["search"]);
    }

    #[test]
    fn resource_urls() {
        let transport = "https://a.example/cfg/manifest.json";
        let url = |resource, kind, id, extra| resource_url(transport, resource, kind, id, extra);
        assert_eq!(
            url("meta", "series", "tt0903747", &[]),
            "https://a.example/cfg/meta/series/tt0903747.json"
        );
        assert_eq!(
            url("stream", "series", "tt0903747:1:1", &[]),
            "https://a.example/cfg/stream/series/tt0903747%3A1%3A1.json"
        );
        assert_eq!(
            url(
                "catalog",
                "movie",
                "top",
                &[("genre", "Sci-Fi"), ("skip", "100")]
            ),
            "https://a.example/cfg/catalog/movie/top/genre=Sci-Fi&skip=100.json"
        );
        // Spaces and ampersands as encodeURIComponent writes them.
        assert_eq!(
            url(
                "catalog",
                "movie",
                "top",
                &[("search", "the long burrow & co")]
            ),
            "https://a.example/cfg/catalog/movie/top/search=the%20long%20burrow%20%26%20co.json"
        );
        // A query on the transport URL stays.
        assert_eq!(
            resource_url(
                "https://b.example/manifest.json?key=1",
                "meta",
                "movie",
                "tt1",
                &[]
            ),
            "https://b.example/meta/movie/tt1.json?key=1"
        );
    }

    #[test]
    fn only_http_transports() {
        let addon = |url: &str| Addon {
            transport_url: url.into(),
            manifest: Manifest::default(),
        };
        assert!(addon("https://a.example/manifest.json?key=1").speaks_http());
        assert!(!addon("https://opensubtitles.strem.io/stremio/v1").speaks_http());
        assert!(!addon("file:///local/manifest.json").speaks_http());
    }

    #[test]
    fn a_full_resource_without_types_serves_nothing() {
        let addon = Addon {
            transport_url: "https://a.example/manifest.json".into(),
            manifest: serde_json::from_value(serde_json::json!({
                "id": "x", "name": "X", "types": ["movie"],
                "resources": [{"name": "stream"}, {"name": "meta", "types": ["movie"], "idPrefixes": []}]
            }))
            .unwrap(),
        };
        assert!(!addon.serves("stream", "movie", "tt1"));
        assert!(
            addon.serves("meta", "movie", "tmdb:5"),
            "empty prefixes: any id"
        );
    }

    #[test]
    fn numbers_where_text_is_expected() {
        let meta: MetaPreview = serde_json::from_value(serde_json::json!({
            "id": "tmdb:1", "type": "movie", "name": null,
            "imdbRating": 7.4, "releaseInfo": 2025, "runtime": "1h 44m",
            "links": [{"name": "Comedy", "category": "Genres", "url": "stremio:///x"}]
        }))
        .unwrap();
        assert_eq!(meta.rating(), Some("7.4"));
        assert_eq!(meta.release_info.as_deref(), Some("2025"));
        assert_eq!(meta.name, "");
        assert_eq!(meta.genre_names(), ["Comedy"]);
    }

    #[test]
    fn a_catalog_page_reads_and_skips_broken_items() {
        let metas: Vec<MetaPreview> = lenient(
            serde_json::from_str::<Value>(&fixture("cinemeta-catalog-movie-top.json"))
                .unwrap()
                .get("metas"),
        );
        assert_eq!(metas.len(), 3);
        assert!(
            metas
                .iter()
                .all(|m| m.kind == "movie" && m.poster.is_some())
        );
        let broken = serde_json::json!([{"id": "tt1", "type": "movie", "name": "Fine"}, {"id": 5}]);
        assert_eq!(lenient::<MetaPreview>(Some(&broken)).len(), 1);
    }

    #[test]
    fn a_series_reads_with_its_episodes_in_order() {
        #[derive(Deserialize)]
        struct Answer {
            meta: MetaItem,
        }
        let meta = serde_json::from_str::<Answer>(&fixture("cinemeta-meta-series.json"))
            .unwrap()
            .meta;
        assert_eq!(meta.preview.name, "Breaking Bad");
        assert_eq!(meta.videos.len(), 67);
        let shown = meta.display_videos();
        assert_eq!(shown[0].id, "tt0903747:1:1");
        assert_eq!(shown[0].title(), "Pilot");
        assert!(!shown[0].overview().is_empty());
        assert_eq!(shown.last().unwrap().season, Some(0), "specials last");
        let counted = meta.bitfield_ids();
        assert!(counted[0].starts_with("tt0903747:0:"), "specials first");
        assert_eq!(counted.len(), 67);

        let now = Timestamp::now();
        assert_eq!(
            meta.next_video("tt0903747:1:7", now).map(|v| v.id.as_str()),
            Some("tt0903747:2:1"),
            "the next season after the last episode"
        );
        let finale = shown.iter().rev().find(|v| v.season != Some(0)).unwrap();
        assert!(
            meta.next_video(&finale.id, now).is_none(),
            "no special after a regular episode"
        );
        let before_it_aired = "2008-01-01T00:00:00Z".parse().unwrap();
        assert!(meta.next_video("tt0903747:1:1", before_it_aired).is_none());
        assert!(meta.preview.rating().is_some());
        assert!(!meta.preview.people("Cast").is_empty());
    }

    #[test]
    fn a_movie_plays_its_default_video() {
        #[derive(Deserialize)]
        struct Answer {
            meta: MetaItem,
        }
        let meta = serde_json::from_str::<Answer>(&fixture("cinemeta-meta-movie.json"))
            .unwrap()
            .meta;
        assert_eq!(meta.movie_video_id(), "tt1375666");
        assert_eq!(meta.preview.runtime.as_deref(), Some("148 min"));
        let bare = MetaItem {
            preview: MetaPreview {
                id: "tt9".into(),
                ..MetaPreview::default()
            },
            videos: vec![],
        };
        assert_eq!(bare.movie_video_id(), "tt9");
    }

    #[test]
    fn subtitles_read() {
        let subtitles: Vec<Subtitle> = lenient(
            serde_json::from_str::<Value>(&fixture("opensubtitles-subtitles-movie.json"))
                .unwrap()
                .get("subtitles"),
        );
        assert!(subtitles.iter().any(|s| s.lang == "eng"));
        assert!(subtitles.iter().all(|s| s.url.starts_with("https://")));
    }

    #[test]
    fn a_stream_with_headers_and_hints() {
        let stream: Stream = serde_json::from_value(serde_json::json!({
            "name": "[TB⚡] AIOStreams\n2160p",
            "description": "The.Long.Burrow.2025.2160p.WEB-DL.DV.HDR10\n💾 18.4 GB",
            "url": "https://aio.example/playback/x",
            "behaviorHints": {
                "bingeGroup": "aio|tb|2160p",
                "filename": "The.Long.Burrow.2025.2160p.WEB-DL.DV.HDR10.mkv",
                "videoSize": 19_756_000_000_u64,
                "proxyHeaders": {"request": {"User-Agent": "aio", "X-Bad": 3}}
            }
        }))
        .unwrap();
        assert!(stream.details().starts_with("The.Long.Burrow"));
        assert_eq!(
            stream.headers(),
            [("User-Agent".to_owned(), "aio".to_owned())]
        );
        assert_eq!(stream.behavior_hints.video_size, Some(19_756_000_000));
        let old: Stream =
            serde_json::from_value(serde_json::json!({"title": "old style", "infoHash": "abc"}))
                .unwrap();
        assert_eq!(old.details(), "old style");
        assert!(old.url.is_none());
    }
}
