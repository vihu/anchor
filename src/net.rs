//! HTTP for the Stremio API and the addons: one way to fetch JSON, and
//! errors that say what went wrong without the URL.
//!
//! Addon URLs carry the addon's configuration (AIOStreams puts its
//! credentials there), so no error from here ever holds one.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Result type for this module.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// How long a request may take, start to end.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The largest answer read, in bytes: a big library is a few megabytes.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// What can go wrong fetching JSON.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The server could not be reached: no network, or no such host.
    Offline,
    /// The server took longer than the timeout.
    Timeout,
    /// The server answered with an HTTP error status.
    Status(u16),
    /// The answer is not the JSON expected.
    Json(serde_json::Error),
    /// Anything else, described without the URL.
    Other(String),
}

/// An HTTP client for JSON, shared by every request.
#[derive(Clone, Debug)]
pub struct Client {
    agent: ureq::Agent,
}

// Public API
impl Client {
    /// A client with anchor's timeout.
    pub fn new() -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("anchor/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent }
    }

    /// Fetches `url` and reads its JSON.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the request fails or the answer is not
    /// the JSON expected.
    pub fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let mut response = self.agent.get(url).call().map_err(Error::from)?;
        read(response.body_mut())
    }

    /// Posts `body` as JSON to `url` and reads the JSON answer.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the request fails or the answer is not
    /// the JSON expected.
    pub fn post<T: DeserializeOwned>(&self, url: &str, body: &impl Serialize) -> Result<T> {
        let mut response = self.agent.post(url).send_json(body).map_err(Error::from)?;
        read(response.body_mut())
    }

    /// Fetches `url` and reads its body, failing past `max_bytes`.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the request fails or the body is larger.
    pub fn bytes(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>> {
        let mut response = self.agent.get(url).call()?;
        Ok(response
            .body_mut()
            .with_config()
            .limit(max_bytes)
            .read_to_vec()?)
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::StatusCode(code) => Error::Status(code),
            ureq::Error::Timeout(_) => Error::Timeout,
            ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => Error::Offline,
            ureq::Error::Json(e) => Error::Json(e),
            ureq::Error::Io(e) => Error::Other(e.kind().to_string()),
            ureq::Error::Tls(what) => Error::Other(format!("secure connection failed: {what}")),
            ureq::Error::Rustls(e) => Error::Other(format!("secure connection failed: {e}")),
            ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => {
                Error::Other("too many redirects".to_owned())
            }
            ureq::Error::BodyExceedsLimit(_) => Error::Other("the answer is too large".to_owned()),
            ureq::Error::BadUri(_) | ureq::Error::Http(_) => {
                Error::Other("not a valid URL".to_owned())
            }
            // These can hold the URL, so say only what kind of failure.
            _ => Error::Other("the request failed".to_owned()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Json(e) => Some(e),
            Error::Offline | Error::Timeout | Error::Status(_) | Error::Other(_) => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Offline => write!(f, "no connection"),
            Error::Timeout => write!(f, "no answer in {} s", TIMEOUT.as_secs()),
            Error::Status(code) => write!(f, "the server answered HTTP {code}"),
            Error::Json(e) => write!(f, "an answer anchor does not understand: {e}"),
            Error::Other(what) => write!(f, "{what}"),
        }
    }
}

/// The items of `list` that parse; the rest are skipped, so one item an
/// addon or the API gets wrong does not cost the whole answer.
pub(crate) fn lenient<T: DeserializeOwned>(list: Option<&serde_json::Value>) -> Vec<T> {
    list.and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| T::deserialize(item).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn read<T: DeserializeOwned>(body: &mut ureq::Body) -> Result<T> {
    let text = body
        .with_config()
        .limit(MAX_BYTES)
        .read_to_string()
        .map_err(Error::from)?;
    serde_json::from_str(&text).map_err(Error::Json)
}
