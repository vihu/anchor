//! The Stremio API client against a stand-in server: the bodies it posts
//! are stremio-core's, the answers it reads are the API's, and refusals
//! (which come as HTTP 200) are errors.

use anchor::api::{Api, Error};
use anchor::net::Client;
use mockito::{Matcher, Server, ServerGuard};
use serde_json::json;

fn api(server: &ServerGuard) -> Api {
    Api::at(Client::new(), &format!("{}/api/", server.url()))
}

#[test]
fn login_posts_the_credentials_and_reads_the_session() {
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/api/login")
        .match_body(Matcher::Json(json!({
            "type": "Login", "email": "alex@example.com", "password": "pw", "facebook": false
        })))
        .with_body(
            json!({"result": {"authKey": "k1", "user": {
                "_id": "u1", "email": "alex@example.com", "fbId": "", "avatar": null,
                "lastModified": "2026-01-01T00:00:00.000Z", "dateRegistered": "2020-01-01T00:00:00.000Z",
                "gdpr_consent": {"tos": true, "privacy": true, "marketing": false}
            }}})
            .to_string(),
        )
        .create();
    let session = api(&server).login("alex@example.com", "pw").unwrap();
    assert_eq!(session.key, "k1");
    assert_eq!(session.user.id, "u1");
    assert_eq!(session.user.email, "alex@example.com");
    mock.assert();
}

#[test]
fn a_refusal_is_an_error_though_the_status_is_200() {
    let mut server = Server::new();
    server
        .mock("POST", "/api/login")
        .with_body(r#"{"error":{"code":2,"message":"User not found","wrongEmail":true}}"#)
        .create();
    match api(&server).login("nobody@example.com", "pw") {
        Err(Error::Api { code, message }) => {
            assert_eq!(code, 2);
            assert_eq!(message, "User not found");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn an_expired_key_says_so() {
    let mut server = Server::new();
    server
        .mock("POST", "/api/addonCollectionGet")
        .with_body(r#"{"error":{"code":1,"message":"Session does not exist"}}"#)
        .create();
    let error = api(&server).addons("old").unwrap_err();
    assert!(error.signed_out(), "{error}");
}

#[test]
fn the_addon_collection_in_order() {
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/api/addonCollectionGet")
        .match_body(Matcher::Json(json!({
            "type": "AddonCollectionGet", "authKey": "k1", "update": true
        })))
        .with_body(
            json!({"result": {"lastModified": "2026-10-01T00:00:00Z", "addons": [
                {"transportUrl": "https://meta.example/cfg/manifest.json", "flags": {"official": false},
                 "manifest": {"id": "aiometadata", "version": "1.9.2", "name": "AIOMetadata",
                    "types": ["movie", "series"], "resources": ["catalog", "meta"],
                    "catalogs": [{"type": "movie", "id": "trending", "name": "Trending"}]}},
                {"transportUrl": "https://streams.example/cfg/manifest.json",
                 "manifest": {"id": "aiostreams", "version": "2.14.0", "name": "AIOStreams",
                    "types": ["movie", "series"], "resources": ["stream"]}},
                {"transportUrl": "https://broken.example/manifest.json", "manifest": {"id": 5}}
            ]}})
            .to_string(),
        )
        .create();
    let addons = api(&server).addons("k1").unwrap();
    let names: Vec<&str> = addons.iter().map(|a| a.manifest.name.as_str()).collect();
    assert_eq!(
        names,
        ["AIOMetadata", "AIOStreams"],
        "the broken one skipped"
    );
    assert!(addons[1].serves("stream", "series", "tt0903747:1:1"));
    mock.assert();
}

#[test]
fn the_library_reads_and_writes_back() {
    let item = json!({
        "_id": "tt1375666", "name": "Inception", "type": "movie", "poster": null,
        "posterShape": "poster", "removed": false, "temp": false,
        "_ctime": "2026-09-01T10:00:00Z", "_mtime": "2026-09-01T10:00:00Z",
        "state": {"lastWatched": "2026-09-01T10:00:00Z", "timeWatched": 0, "timeOffset": 3600000,
            "overallTimeWatched": 0, "timesWatched": 0, "flaggedWatched": 0, "duration": 8880000,
            "video_id": "tt1375666", "watched": null, "noNotif": false},
        "behaviorHints": {"defaultVideoId": "tt1375666", "featuredVideoId": null, "hasScheduledVideos": false}
    });
    let mut server = Server::new();
    server
        .mock("POST", "/api/datastoreGet")
        .match_body(Matcher::Json(json!({
            "authKey": "k1", "collection": "libraryItem", "ids": [], "all": true
        })))
        .with_body(json!({"result": [item, {"_id": "broken"}]}).to_string())
        .create();
    let put = server
        .mock("POST", "/api/datastorePut")
        .match_body(Matcher::Json(json!({
            "authKey": "k1", "collection": "libraryItem", "changes": [item]
        })))
        .with_body(r#"{"result":{"success":true}}"#)
        .create();
    let api = api(&server);
    let library = api.library("k1").unwrap();
    assert_eq!(library.len(), 1, "the broken item skipped");
    assert_eq!(library[0].resume_at("tt1375666"), Some(3600.0));
    // Read and written unchanged: the same JSON goes back.
    api.put_library("k1", &library).unwrap();
    put.assert();
}

#[test]
fn library_items_by_id() {
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/api/datastoreGet")
        .match_body(Matcher::Json(json!({
            "authKey": "k1", "collection": "libraryItem", "ids": ["tt1"], "all": false
        })))
        .with_body(r#"{"result":[]}"#)
        .create();
    assert!(
        api(&server)
            .library_items("k1", &["tt1"])
            .unwrap()
            .is_empty()
    );
    mock.assert();
}

#[test]
fn logout() {
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/api/logout")
        .match_body(Matcher::Json(json!({"type": "Logout", "authKey": "k1"})))
        .with_body(r#"{"result":{"success":true}}"#)
        .create();
    api(&server).logout("k1").unwrap();
    mock.assert();
}

#[test]
fn an_unreachable_api_is_offline() {
    let api = Api::at(Client::new(), "http://127.0.0.1:9/api/");
    assert!(matches!(api.login("a", "b"), Err(Error::Net(_))));
}
