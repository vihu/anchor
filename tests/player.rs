//! The player against `fake-mpv.py`, a stand-in that serves mpv's IPC
//! socket: progress arrives, the arguments reach mpv, and each way a
//! playback ends is reported as such.

use std::sync::mpsc;
use std::time::Duration;

use anchor::player::{Event, Launch, Outcome, Playback, Progress};
use camino::Utf8PathBuf;

/// How long a fake playback may take.
const TIMEOUT: Duration = Duration::from_secs(20);

fn fake_mpv() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake-mpv.py")
}

fn launch() -> Launch {
    Launch {
        url: "https://debrid.example/dl/1".into(),
        title: "The Long Burrow".into(),
        start: Some(30.0),
        ..Launch::default()
    }
}

/// Plays `launch` in the fake mpv, ending as `end` says; the arguments it
/// got are written into the returned directory's `args`.
fn play(end: &str, launch: &Launch) -> (Playback, mpsc::Receiver<Event>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut launch = launch.clone();
    launch.extra_args.push(format!("--fake-end={end}"));
    launch
        .extra_args
        .push(format!("--fake-args={}", dir.path().join("args").display()));
    let (tx, rx) = mpsc::channel();
    let playback = Playback::start(&fake_mpv(), &launch, move |event| {
        let _ = tx.send(event);
    })
    .unwrap();
    (playback, rx, dir)
}

/// Collects events until the playback ends; returns the progress reports
/// and the end.
fn until_end(rx: &mpsc::Receiver<Event>) -> (Vec<Progress>, Progress, Outcome) {
    let mut progress = Vec::new();
    loop {
        match rx.recv_timeout(TIMEOUT).expect("the playback ends") {
            Event::Progress(p) => progress.push(p),
            Event::Ended(last, outcome) => return (progress, last, outcome),
        }
    }
}

#[test]
fn plays_to_the_end() {
    let (_playback, rx, dir) = play("eof", &launch());
    let (progress, last, outcome) = until_end(&rx);
    assert_eq!(outcome, Outcome::Finished);
    assert_eq!(last.duration, 100.0);
    assert_eq!(last.position, 100.0);
    assert!(
        progress
            .iter()
            .any(|p| p.position >= 30.0 && p.duration == 100.0),
        "{progress:?}"
    );
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert!(args.contains("--start=30.0\n"), "{args}");
    assert!(
        args.ends_with("--\nhttps://debrid.example/dl/1\n"),
        "{args}"
    );
}

#[test]
fn stop_quits_mpv() {
    let (playback, rx, _dir) = play("quit", &launch());
    // Wait until it plays, then stop it.
    loop {
        if let Event::Progress(p) = rx.recv_timeout(TIMEOUT).unwrap()
            && p.position > 31.0
        {
            break;
        }
    }
    playback.stop();
    let (_, last, outcome) = until_end(&rx);
    assert_eq!(outcome, Outcome::Stopped);
    assert!(last.position > 31.0, "{last:?}");
}

#[test]
fn a_refused_link_fails_with_the_http_status() {
    let (_playback, rx, _dir) = play("error", &launch());
    let (_, _, outcome) = until_end(&rx);
    assert_eq!(
        outcome,
        Outcome::Failed("the server answered HTTP 403 Forbidden".into())
    );
}

#[test]
fn mpv_that_exits_at_once_fails() {
    let (_playback, rx, _dir) = play("early", &launch());
    let (progress, _, outcome) = until_end(&rx);
    assert!(progress.is_empty());
    assert_eq!(outcome, Outcome::Failed("mpv could not open it".into()));
}

#[test]
fn a_missing_mpv_is_an_error() {
    let missing = Utf8PathBuf::from("/nonexistent/mpv");
    assert!(Playback::start(&missing, &launch(), |_| {}).is_err());
}

/// The real mpv on a two-second generated picture, without a window or
/// sound: `cargo test -- --ignored` where mpv is installed.
#[test]
#[ignore = "needs mpv"]
fn real_mpv_reports_progress_and_the_end() {
    let mpv = anchor::player::find_mpv(None).expect("mpv is installed");
    let launch = Launch {
        url: "av://lavfi:testsrc=duration=2:rate=10".into(),
        title: "test".into(),
        extra_args: vec![
            "--vo=null".into(),
            "--ao=null".into(),
            "--force-window=no".into(),
        ],
        ..Launch::default()
    };
    let (tx, rx) = mpsc::channel();
    let _playback = Playback::start(&mpv, &launch, move |event| {
        let _ = tx.send(event);
    })
    .unwrap();
    let (progress, last, outcome) = until_end(&rx);
    assert_eq!(outcome, Outcome::Finished);
    assert!((last.duration - 2.0).abs() < 0.5, "{last:?}");
    assert!(!progress.is_empty());
}
