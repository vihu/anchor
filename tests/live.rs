//! The addon client against real public addons, Cinemeta and OpenSubtitles:
//! `cargo test --test live -- --ignored` (needs the network).

use anchor::addon::{Addon, Addons, Manifest};
use anchor::net::Client;

fn addon(url: &str) -> Addon {
    let manifest: Manifest = Client::new().get(url).expect("the manifest loads");
    Addon {
        transport_url: url.to_owned(),
        manifest,
    }
}

#[test]
#[ignore = "needs the network"]
fn cinemeta_catalog_meta_and_search() {
    let cinemeta = addon("https://v3-cinemeta.strem.io/manifest.json");
    let addons = Addons::new(Client::new());
    let top = cinemeta
        .manifest
        .catalogs
        .iter()
        .find(|c| c.kind == "movie" && c.browsable())
        .expect("a movie catalog");
    let page = addons.catalog(&cinemeta, top, &[]).unwrap();
    assert!(page.len() >= 20, "{}", page.len());
    let next = addons.catalog(&cinemeta, top, &[("skip", "100")]).unwrap();
    assert!(!next.is_empty());
    assert_ne!(page[0].id, next[0].id, "skip gives the next page");

    let found = addons
        .catalog(&cinemeta, top, &[("search", "breaking bad")])
        .unwrap();
    assert!(!found.is_empty());

    let series = addons.meta(&cinemeta, "series", "tt0903747").unwrap();
    assert_eq!(series.preview.name, "Breaking Bad");
    assert!(series.videos.len() > 60);
    let genre = top.genres().first().expect("genres").clone();
    let filtered = addons
        .catalog(&cinemeta, top, &[("genre", genre.as_str())])
        .unwrap();
    assert!(!filtered.is_empty());
}

#[test]
#[ignore = "needs the network"]
fn opensubtitles_for_a_movie_and_an_episode() {
    let subs = addon("https://opensubtitles-v3.strem.io/manifest.json");
    let addons = Addons::new(Client::new());
    assert!(subs.serves("subtitles", "series", "tt0903747:1:1"));
    let movie = addons.subtitles(&subs, "movie", "tt1375666", &[]).unwrap();
    assert!(movie.iter().any(|s| s.lang == "eng"));
    let episode = addons
        .subtitles(
            &subs,
            "series",
            "tt0903747:1:1",
            &[("filename", "Breaking.Bad.S01E01.720p.mkv")],
        )
        .unwrap();
    assert!(!episode.is_empty());
}
