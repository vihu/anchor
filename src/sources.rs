//! Which of the account's addons answer what: the catalogs the sidebar and
//! Home list, the catalogs a search asks, and the addons asked for a title,
//! its streams and its subtitles.
//!
//! The account's order decides, as in Stremio: the first addon that can
//! answer a title's metadata does; every addon that can is asked for
//! streams and subtitles.

use crate::addon::{Addon, Catalog};

/// The content types the sidebar lists first, with their headings.
const KNOWN_TYPES: [(&str, &str); 2] = [("movie", "Movies"), ("series", "Series")];

/// The account's addons that anchor can ask, in the account's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    addons: Vec<Addon>,
}

/// A catalog of one of the addons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CatalogRef {
    /// The addon's place in the account's order.
    pub addon: usize,
    /// The catalog's place in the addon's manifest.
    pub catalog: usize,
}

/// A sidebar section: one content type and its catalogs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// The content type, for example `movie`.
    pub kind: String,
    /// Its heading, for example `Movies`.
    pub title: String,
    /// Its catalogs, in the account's order.
    pub catalogs: Vec<CatalogRef>,
}

// Public API
impl Sources {
    /// The addons anchor can ask: those that speak HTTP, in `addons`' order.
    pub fn new(addons: Vec<Addon>) -> Self {
        Self {
            addons: addons.into_iter().filter(Addon::speaks_http).collect(),
        }
    }

    /// The addons, in the account's order.
    pub fn addons(&self) -> &[Addon] {
        &self.addons
    }

    /// The addon and catalog `at` names, if it still exists.
    pub fn catalog(&self, at: CatalogRef) -> Option<(&Addon, &Catalog)> {
        let addon = self.addons.get(at.addon)?;
        Some((addon, addon.manifest.catalogs.get(at.catalog)?))
    }

    /// Every catalog that lists without an extra, in the account's order:
    /// Home's rows.
    pub fn browsable(&self) -> Vec<CatalogRef> {
        self.catalogs(Catalog::browsable)
    }

    /// Every catalog that can be searched, in the account's order.
    pub fn searchable(&self) -> Vec<CatalogRef> {
        self.catalogs(Catalog::searchable)
    }

    /// The sidebar's sections: movies, then series, then any other type in
    /// the order it first appears, each with its browsable catalogs.
    pub fn sections(&self) -> Vec<Section> {
        let mut sections: Vec<Section> = KNOWN_TYPES
            .iter()
            .map(|(kind, title)| Section {
                kind: (*kind).to_owned(),
                title: (*title).to_owned(),
                catalogs: Vec::new(),
            })
            .collect();
        for at in self.browsable() {
            let (_, catalog) = self.catalog(at).expect("browsable lists existing catalogs");
            match sections.iter_mut().find(|s| s.kind == catalog.kind) {
                Some(section) => section.catalogs.push(at),
                None => sections.push(Section {
                    kind: catalog.kind.clone(),
                    title: heading(&catalog.kind),
                    catalogs: vec![at],
                }),
            }
        }
        sections.retain(|s| !s.catalogs.is_empty());
        sections
    }

    /// The addons that can answer `resource` for `kind` and `id`, in the
    /// account's order.
    pub fn serving(&self, resource: &str, kind: &str, id: &str) -> Vec<&Addon> {
        self.addons
            .iter()
            .filter(|a| a.serves(resource, kind, id))
            .collect()
    }
}

// Private API
impl Sources {
    fn catalogs(&self, keep: impl Fn(&Catalog) -> bool) -> Vec<CatalogRef> {
        self.addons
            .iter()
            .enumerate()
            .flat_map(|(addon, a)| {
                a.manifest
                    .catalogs
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| keep(c))
                    .map(move |(catalog, _)| CatalogRef { addon, catalog })
            })
            .collect()
    }
}

/// A heading for a content type anchor does not know, for example `anime`
/// or `anime.series` as `Anime` and `Anime series`.
fn heading(kind: &str) -> String {
    let words = kind.replace(['.', '_', '-'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addon(url: &str, manifest: serde_json::Value) -> Addon {
        Addon {
            transport_url: url.into(),
            manifest: serde_json::from_value(manifest).unwrap(),
        }
    }

    fn sources() -> Sources {
        Sources::new(vec![
            addon(
                "https://meta.example/cfg/manifest.json",
                serde_json::json!({
                    "id": "aiometadata", "name": "AIOMetadata", "types": ["movie", "series", "anime"],
                    "resources": ["catalog", "meta"], "idPrefixes": ["tt", "tmdb:", "kitsu:"],
                    "catalogs": [
                        {"type": "series", "id": "trending", "name": "Trending"},
                        {"type": "anime", "id": "airing", "name": "Airing"},
                        {"type": "movie", "id": "trending", "name": "Trending",
                         "extra": [{"name": "genre", "options": ["Action"]}, {"name": "skip"}]},
                        {"type": "movie", "id": "search", "name": "Search",
                         "extra": [{"name": "search", "isRequired": true}]}
                    ]
                }),
            ),
            addon(
                "https://opensubtitles.strem.io/stremio/v1",
                serde_json::json!({
                    "id": "legacy", "name": "Legacy", "types": ["movie"], "resources": ["subtitles"]
                }),
            ),
            addon(
                "https://v3-cinemeta.strem.io/manifest.json",
                serde_json::json!({
                    "id": "cinemeta", "name": "Cinemeta", "types": ["movie", "series"],
                    "resources": ["catalog", "meta"], "idPrefixes": ["tt"],
                    "catalogs": [{"type": "movie", "id": "top", "name": "Popular",
                        "extra": [{"name": "search"}, {"name": "skip"}]}]
                }),
            ),
            addon(
                "https://streams.example/cfg/manifest.json",
                serde_json::json!({
                    "id": "aiostreams", "name": "AIOStreams", "types": ["movie", "series"],
                    "resources": ["stream"], "idPrefixes": ["tt", "tmdb:"]
                }),
            ),
        ])
    }

    #[test]
    fn legacy_addons_are_left_out() {
        let sources = sources();
        let names: Vec<&str> = sources
            .addons()
            .iter()
            .map(|a| a.manifest.name.as_str())
            .collect();
        assert_eq!(names, ["AIOMetadata", "Cinemeta", "AIOStreams"]);
    }

    #[test]
    fn sections_are_movies_series_then_the_rest() {
        let sources = sources();
        let sections = sources.sections();
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Movies", "Series", "Anime"]);
        let movie_catalogs: Vec<&str> = sections[0]
            .catalogs
            .iter()
            .map(|at| sources.catalog(*at).unwrap().1.title())
            .collect();
        assert_eq!(
            movie_catalogs,
            ["Trending", "Popular"],
            "not the search-only one"
        );
    }

    #[test]
    fn search_asks_every_searchable_catalog() {
        let sources = sources();
        let searched: Vec<(&str, &str)> = sources
            .searchable()
            .into_iter()
            .map(|at| {
                let (addon, catalog) = sources.catalog(at).unwrap();
                (addon.manifest.id.as_str(), catalog.id.as_str())
            })
            .collect();
        assert_eq!(searched, [("aiometadata", "search"), ("cinemeta", "top")]);
    }

    #[test]
    fn who_answers_a_title() {
        let sources = sources();
        let meta: Vec<&str> = sources
            .serving("meta", "movie", "tmdb:603")
            .iter()
            .map(|a| a.manifest.id.as_str())
            .collect();
        assert_eq!(meta, ["aiometadata"], "Cinemeta does not do tmdb ids");
        assert_eq!(sources.serving("meta", "movie", "tt0133093").len(), 2);
        assert_eq!(
            sources.serving("stream", "series", "tt0903747:1:1").len(),
            1
        );
        assert!(sources.serving("stream", "anime", "kitsu:1:1").is_empty());
    }

    #[test]
    fn headings_for_other_types() {
        assert_eq!(heading("anime"), "Anime");
        assert_eq!(heading("anime.series"), "Anime series");
        assert_eq!(heading(""), "");
    }
}
