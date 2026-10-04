//! The user's own mpv, run as a process of its own: anchor finds it, starts
//! it on a stream, and follows the position over mpv's JSON IPC socket.
//!
//! anchor draws no video. mpv opens its own window with the user's
//! `mpv.conf`, scripts and shaders, so picture (Dolby Vision included),
//! sound and on-screen controls are mpv's. Stream URLs and their headers
//! carry debrid keys, so none is ever logged or put in an error, nor put on
//! mpv's command line, which other users can read: mpv starts idle and gets
//! them over its socket.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

/// Where mpv usually is on macOS, for an app started from Finder, whose
/// `PATH` has no Homebrew in it.
const MACOS_PLACES: [&str; 3] = [
    "/opt/homebrew/bin/mpv",
    "/usr/local/bin/mpv",
    "/Applications/mpv.app/Contents/MacOS/mpv",
];
/// The IPC command that quits mpv.
const QUIT: &str = "{\"command\":[\"quit\"]}\n";
/// How long mpv gets to open its IPC socket, and to quit when asked.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Between attempts to connect to the socket, and between checks on the
/// process.
const POLL: Duration = Duration::from_millis(100);
/// Least time between two progress reports while the position moves.
const REPORT_EVERY: Duration = Duration::from_millis(500);
/// Lines of mpv's error output kept to explain a failure.
const STDERR_LINES: usize = 40;
/// Ids for the properties anchor observes.
const TIME_POS: u64 = 1;
const DURATION: u64 = 2;
const PAUSE: u64 = 3;

/// A stream to play, and how.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Launch {
    /// The stream URL.
    pub url: String,
    /// The window title, for example `Small Thieves · S2 E4 The Pantry Job`.
    pub title: String,
    /// Where to start, in seconds; `None` from the beginning.
    pub start: Option<f64>,
    /// Subtitle files mpv adds as extra tracks.
    pub subtitles: Vec<String>,
    /// HTTP headers the stream needs, as name and value.
    pub headers: Vec<(String, String)>,
    /// The user's extra arguments, added after anchor's own.
    pub extra_args: Vec<String>,
}

/// What mpv reports while it plays.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    /// Seconds into the stream.
    pub position: f64,
    /// The stream's length in seconds; 0 while unknown.
    pub duration: f64,
    /// Paused by the user.
    pub paused: bool,
}

/// Something that happened to the mpv anchor started.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The position, the length or the pause state changed.
    Progress(Progress),
    /// mpv exited; the last progress it reported and why it ended.
    Ended(Progress, Outcome),
}

/// How a playback ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The stream played to its end.
    Finished,
    /// The user quit mpv, or anchor stopped it.
    Stopped,
    /// mpv could not play the stream; why, without the URL.
    Failed(String),
}

/// An mpv anchor started, until it exits.
pub struct Playback {
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<UnixStream>>>,
}

/// What can go wrong starting mpv.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The mpv program could not be started.
    Spawn(Utf8PathBuf, std::io::Error),
}

// Public API
impl Playback {
    /// Starts `mpv` on `launch`; `events` runs on a worker thread for each
    /// [`Event`], the last being [`Event::Ended`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Spawn`] when `mpv` cannot be started.
    pub fn start(
        mpv: &Utf8Path,
        launch: &Launch,
        events: impl Fn(Event) + Send + 'static,
    ) -> Result<Self, Error> {
        let socket_path = socket_path();
        let _ = std::fs::remove_file(&socket_path);
        let load = load_commands(launch);
        let mut child = Command::new(mpv)
            .args(args(launch, &socket_path))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Spawn(mpv.to_owned(), e))?;
        let stderr = child.stderr.take().map(keep_tail);
        let playback = Self {
            stop: Arc::new(AtomicBool::new(false)),
            socket: Arc::new(Mutex::new(None)),
        };
        let (stop, socket) = (Arc::clone(&playback.stop), Arc::clone(&playback.socket));
        thread::spawn(move || {
            supervise(child, &socket_path, &load, &stop, &socket, stderr, &events);
        });
        Ok(playback)
    }

    /// Asks mpv to quit; it reports [`Outcome::Stopped`] once it has.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(stream) = self
            .socket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            let _ = stream.write_all(QUIT.as_bytes());
        }
    }
}

/// Finds mpv: `configured` when it is set and exists, else `mpv` on the
/// `PATH`, else (on macOS) where Homebrew and mpv.app put it.
pub fn find_mpv(configured: Option<&Utf8Path>) -> Option<Utf8PathBuf> {
    if let Some(path) = configured.filter(|p| !p.as_str().is_empty()) {
        return path.is_file().then(|| path.to_owned());
    }
    if let Some(path) = which::which("mpv")
        .ok()
        .and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
    {
        return Some(path);
    }
    if cfg!(target_os = "macos") {
        return MACOS_PLACES
            .iter()
            .map(Utf8Path::new)
            .find(|p| p.is_file())
            .map(Utf8Path::to_owned);
    }
    None
}

/// mpv's version, for example `0.41.0`; `None` when it does not run.
pub fn version(mpv: &Utf8Path) -> Option<String> {
    let output = Command::new(mpv)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_version(&text)
}

/// The arguments anchor starts mpv with, the user's extra ones after its
/// own. No stream URL and no header: [`load_commands`] sends those.
pub fn args(launch: &Launch, socket: &Utf8Path) -> Vec<String> {
    let mut args = vec![
        format!("--input-ipc-server={socket}"),
        // Wait for the stream over the socket, and quit after it.
        "--idle=once".to_owned(),
        // A window at once: a debrid link can take seconds to open.
        "--force-window=immediate".to_owned(),
        format!("--force-media-title={}", launch.title),
    ];
    if let Some(start) = launch.start.filter(|s| *s > 0.0) {
        args.push(format!("--start={start:.1}"));
    }
    // The -append form takes one value each, so commas and colons in a URL
    // are not split.
    for subtitle in &launch.subtitles {
        args.push(format!("--sub-files-append={subtitle}"));
    }
    args.extend(launch.extra_args.iter().cloned());
    args
}

/// The IPC commands that hand mpv the stream: its headers, then the URL.
pub fn load_commands(launch: &Launch) -> Vec<String> {
    let mut commands = Vec::new();
    if !launch.headers.is_empty() {
        let fields: Vec<String> = launch
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();
        commands
            .push(serde_json::json!({"command": ["set_property", "http-header-fields", fields]}));
    }
    commands.push(serde_json::json!({"command": ["loadfile", launch.url]}));
    commands.iter().map(|c| format!("{c}\n")).collect()
}

/// Splits the user's extra arguments as a shell would; `None` when the
/// quoting is unbalanced.
pub fn split_args(text: &str) -> Option<Vec<String>> {
    shlex::split(text)
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Spawn(_, e) => Some(e),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Spawn(path, e) => write!(f, "could not start {path}: {e}"),
        }
    }
}

/// Where this run's IPC socket goes: the runtime directory on Linux, the
/// temporary one elsewhere, with a name no other run uses.
fn socket_path() -> Utf8PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map_or_else(
            || Utf8PathBuf::from_path_buf(std::env::temp_dir()).unwrap_or_else(|_| "/tmp".into()),
            Utf8PathBuf::from,
        );
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("anchor-mpv-{}-{n}.sock", std::process::id()))
}

/// The version from `mpv --version`'s first line, `mpv v0.41.0 Copyright…`.
fn parse_version(text: &str) -> Option<String> {
    let word = text.split_whitespace().nth(1)?;
    let version = word.strip_prefix('v').unwrap_or(word);
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| version.to_owned())
}

/// What mpv reported, for the end.
#[derive(Default)]
struct Shared {
    progress: Progress,
    /// The reason of mpv's last `end-file`, and its error if any.
    end: Option<(String, Option<String>)>,
}

/// Follows mpv until it exits, then reports how it ended.
fn supervise(
    mut child: Child,
    socket_path: &Utf8Path,
    load: &[String],
    stop: &AtomicBool,
    socket: &Mutex<Option<UnixStream>>,
    stderr: Option<thread::JoinHandle<Vec<String>>>,
    events: &(dyn Fn(Event) + Send),
) {
    let mut shared = Shared::default();
    match connect(socket_path, &mut child, stop) {
        Some(mut stream) => {
            if let Ok(clone) = stream.try_clone() {
                *socket.lock().unwrap_or_else(PoisonError::into_inner) = Some(clone);
            }
            // A stop that came before the socket was kept never reached
            // mpv: it goes now, instead of the stream.
            let first: Vec<String> = if stop.load(Ordering::Relaxed) {
                vec![QUIT.to_owned()]
            } else {
                load.to_vec()
            };
            if first.iter().all(|c| stream.write_all(c.as_bytes()).is_ok()) {
                follow(stream, &mut shared, stop, events);
            }
        }
        // No socket: an idle mpv would wait for ever.
        None => {
            let _ = child.kill();
        }
    }
    let status = wait(&mut child, stop);
    socket.lock().unwrap_or_else(PoisonError::into_inner).take();
    let _ = std::fs::remove_file(socket_path);
    let lines = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
    let outcome = outcome(
        shared.end.as_ref(),
        status,
        &lines,
        stop.load(Ordering::Relaxed),
    );
    events(Event::Ended(shared.progress, outcome));
}

/// Connects to mpv's socket once it is there; `None` when mpv exits first,
/// anchor stops it, or it never opens one.
fn connect(path: &Utf8Path, child: &mut Child, stop: &AtomicBool) -> Option<UnixStream> {
    let since = Instant::now();
    loop {
        if let Ok(stream) = UnixStream::connect(path) {
            return Some(stream);
        }
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            return None;
        }
        if child.try_wait().ok().flatten().is_some() || since.elapsed() > CONNECT_TIMEOUT {
            return None;
        }
        thread::sleep(POLL);
    }
}

/// Waits for mpv to exit, killing it when anchor stopped it and it does
/// not quit by itself.
fn wait(child: &mut Child, stop: &AtomicBool) -> Option<ExitStatus> {
    let mut stopped_at = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if stop.load(Ordering::Relaxed) {
            let since = *stopped_at.get_or_insert_with(Instant::now);
            if since.elapsed() > CONNECT_TIMEOUT {
                let _ = child.kill();
            }
        }
        thread::sleep(POLL);
    }
}

/// One message from mpv: a reply or an event.
#[derive(Deserialize)]
struct Message {
    event: Option<String>,
    id: Option<u64>,
    data: Option<serde_json::Value>,
    reason: Option<String>,
    file_error: Option<String>,
}

/// Observes the position, length and pause state, and reports them until
/// mpv closes the socket.
///
/// The position moves every frame, so it is reported at most every
/// [`REPORT_EVERY`]; one held back is reported once mpv goes quiet, so a
/// pause shows where it stopped.
fn follow(
    mut stream: UnixStream,
    state: &mut Shared,
    stop: &AtomicBool,
    events: &(dyn Fn(Event) + Send),
) {
    for (id, name) in [
        (TIME_POS, "time-pos"),
        (DURATION, "duration"),
        (PAUSE, "pause"),
    ] {
        let command = format!("{{\"command\":[\"observe_property\",{id},\"{name}\"]}}\n");
        if stream.write_all(command.as_bytes()).is_err() {
            return;
        }
    }
    let _ = stream.set_read_timeout(Some(REPORT_EVERY));
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let mut reported = Instant::now() - REPORT_EVERY;
    let mut held = false;
    let mut stopped_at: Option<Instant> = None;
    loop {
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => return,
            Ok(_) => {}
            // Quiet: what was held back goes out; a partial line stays.
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                // Asked to quit and still talking: give up on it, and let
                // the supervisor kill it.
                if stop.load(Ordering::Relaxed)
                    && stopped_at.get_or_insert_with(Instant::now).elapsed() > CONNECT_TIMEOUT
                {
                    return;
                }
                if held {
                    held = false;
                    reported = Instant::now();
                    events(Event::Progress(state.progress));
                }
                continue;
            }
            Err(_) => return,
        }
        let message = serde_json::from_slice::<Message>(&line);
        line.clear();
        let Ok(message) = message else {
            continue;
        };
        match message.event.as_deref() {
            Some("property-change") => {
                let number = message.data.as_ref().and_then(serde_json::Value::as_f64);
                let urgent = match (message.id, number) {
                    (Some(TIME_POS), Some(position)) => {
                        state.progress.position = position;
                        false
                    }
                    (Some(DURATION), Some(duration)) => {
                        state.progress.duration = duration;
                        true
                    }
                    (Some(PAUSE), _) => {
                        state.progress.paused = message
                            .data
                            .as_ref()
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        true
                    }
                    _ => continue,
                };
                if urgent || reported.elapsed() >= REPORT_EVERY {
                    held = false;
                    reported = Instant::now();
                    events(Event::Progress(state.progress));
                } else {
                    held = true;
                }
            }
            Some("end-file") => {
                state.end = Some((message.reason.unwrap_or_default(), message.file_error));
            }
            _ => {}
        }
    }
}

/// Keeps the last lines of mpv's error output, on a thread of its own so
/// the pipe never fills.
fn keep_tail(stderr: impl Read + Send + 'static) -> thread::JoinHandle<Vec<String>> {
    thread::spawn(move || {
        let mut lines = Vec::new();
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::new();
        // Bytes, not text: a line that is not UTF-8 must not stop the
        // draining, or mpv blocks on a full pipe.
        while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            if lines.len() == STDERR_LINES {
                lines.remove(0);
            }
            lines.push(String::from_utf8_lossy(&line).trim_end().to_owned());
            line.clear();
        }
        lines
    })
}

/// Why mpv ended, from its last `end-file`, its exit status and its error
/// output.
fn outcome(
    end: Option<&(String, Option<String>)>,
    status: Option<ExitStatus>,
    stderr: &[String],
    stopped: bool,
) -> Outcome {
    match end.map(|(reason, error)| (reason.as_str(), error.as_deref())) {
        Some(("eof", _)) => Outcome::Finished,
        Some(("error", error)) => Outcome::Failed(failure(error, stderr)),
        Some(_) => Outcome::Stopped,
        // No end-file: mpv never got as far as the stream.
        None if stopped => Outcome::Stopped,
        None if status.is_some_and(|s| s.success()) => Outcome::Stopped,
        None => Outcome::Failed(failure(None, stderr)),
    }
}

/// Why the stream failed, in words without the URL: an HTTP error when
/// mpv printed one, else mpv's own reason.
fn failure(error: Option<&str>, stderr: &[String]) -> String {
    let http = stderr.iter().rev().find_map(|line| http_error(line));
    match (http, error) {
        (Some(http), _) => format!("the server answered {http}"),
        (None, Some(error)) if !error.is_empty() => format!("mpv: {error}"),
        _ => "mpv could not open it".to_owned(),
    }
}

/// `HTTP error 403 Forbidden` from a line of mpv's output, where FFmpeg
/// reports it; the URL around it is dropped.
fn http_error(line: &str) -> Option<String> {
    let rest = &line[line.find("HTTP error ")? + "HTTP error ".len()..];
    let code: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if code.len() != 3 {
        return None;
    }
    let words: Vec<&str> = rest[code.len()..]
        .split_whitespace()
        .take_while(|w| w.chars().all(char::is_alphabetic))
        .collect();
    Some(if words.is_empty() {
        format!("HTTP {code}")
    } else {
        format!("HTTP {code} {}", words.join(" "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_keep_the_stream_off_the_command_line() {
        let launch = Launch {
            url: "https://debrid.example/dl/abc,def".into(),
            title: "Small Thieves · S2 E4".into(),
            start: Some(1830.25),
            subtitles: vec!["https://subs.example/1.srt".into()],
            headers: vec![("Referer".into(), "https://a.example/x,y".into())],
            extra_args: vec!["--fs".into()],
        };
        let args = args(&launch, Utf8Path::new("/run/user/1000/anchor.sock"));
        assert_eq!(
            args,
            [
                "--input-ipc-server=/run/user/1000/anchor.sock",
                "--idle=once",
                "--force-window=immediate",
                "--force-media-title=Small Thieves · S2 E4",
                "--start=1830.2",
                "--sub-files-append=https://subs.example/1.srt",
                "--fs",
            ]
        );
        // The URL and the headers carry keys: over the socket only.
        assert_eq!(
            load_commands(&launch),
            [
                "{\"command\":[\"set_property\",\"http-header-fields\",[\"Referer: https://a.example/x,y\"]]}\n",
                "{\"command\":[\"loadfile\",\"https://debrid.example/dl/abc,def\"]}\n",
            ]
        );
    }

    #[test]
    fn no_start_from_the_beginning() {
        let launch = Launch {
            url: "u".into(),
            start: Some(0.0),
            ..Launch::default()
        };
        let args = args(&launch, Utf8Path::new("/s"));
        assert!(!args.iter().any(|a| a.starts_with("--start")), "{args:?}");
    }

    #[test]
    fn extra_args_split_like_a_shell() {
        assert_eq!(
            split_args("--fs --profile='my profile'").unwrap(),
            ["--fs", "--profile=my profile"]
        );
        assert_eq!(split_args("").unwrap(), Vec::<String>::new());
        assert!(split_args("--title='open").is_none());
    }

    #[test]
    fn version_from_the_first_line() {
        assert_eq!(
            parse_version("mpv v0.41.0 Copyright © 2000-2025 mpv/MPlayer/mplayer2 projects\n"),
            Some("0.41.0".into())
        );
        assert_eq!(
            parse_version("mpv 0.38.0-dirty Copyright"),
            Some("0.38.0-dirty".into())
        );
        assert_eq!(parse_version("something else"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn http_errors_without_the_url() {
        let line = "[ffmpeg] https: HTTP error 403 Forbidden https://debrid.example/key/abc";
        assert_eq!(http_error(line).as_deref(), Some("HTTP 403 Forbidden"));
        assert_eq!(
            http_error("[ffmpeg] http: HTTP error 404 Not Found").as_deref(),
            Some("HTTP 404 Not Found")
        );
        assert_eq!(http_error("HTTP error 5xx"), None);
        assert_eq!(http_error("Playing: https://x"), None);
    }

    #[test]
    fn outcomes() {
        let ok = std::process::Command::new("true").status().ok();
        let bad = std::process::Command::new("false").status().ok();
        let eof = ("eof".to_owned(), None);
        let quit = ("quit".to_owned(), None);
        let error = ("error".to_owned(), Some("loading failed".to_owned()));
        assert_eq!(outcome(Some(&eof), ok, &[], false), Outcome::Finished);
        assert_eq!(outcome(Some(&quit), ok, &[], false), Outcome::Stopped);
        assert_eq!(
            outcome(Some(&error), bad, &[], false),
            Outcome::Failed("mpv: loading failed".into())
        );
        let stderr = ["[ffmpeg] https: HTTP error 403 Forbidden".to_owned()];
        assert_eq!(
            outcome(Some(&error), bad, &stderr, false),
            Outcome::Failed("the server answered HTTP 403 Forbidden".into())
        );
        assert_eq!(outcome(None, bad, &[], true), Outcome::Stopped);
        assert_eq!(
            outcome(None, bad, &[], false),
            Outcome::Failed("mpv could not open it".into())
        );
    }

    #[test]
    fn find_mpv_prefers_the_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.path().join("my-mpv")).unwrap();
        std::fs::write(&path, "").unwrap();
        assert_eq!(find_mpv(Some(&path)), Some(path.clone()));
        let missing = path.with_file_name("gone");
        assert_eq!(
            find_mpv(Some(&missing)),
            None,
            "a set path is not second-guessed"
        );
    }
}
