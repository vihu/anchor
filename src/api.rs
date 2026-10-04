//! The Stremio API at `api.strem.io`: signing in and out, the account's
//! addons, and its library, as stremio-core calls it
//! (`src/types/api/request.rs`, `response.rs`).
//!
//! Every call is a POST of JSON to `{base}{method}`. The API answers HTTP
//! 200 either way, with `{"result": …}` or `{"error": {"code", "message"}}`.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::addon::Addon;
use crate::library::LibraryItem;
use crate::net::{self, Client, lenient};

/// Result type for this module.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Where the Stremio API is.
pub const API_URL: &str = "https://api.strem.io/api/";
/// The datastore collection of library items.
const LIBRARY: &str = "libraryItem";

/// A Stremio API client.
#[derive(Clone, Debug)]
pub struct Api {
    client: Client,
    base: String,
}

/// A signed-in session: the auth key and whose it is.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Session {
    /// The key every later call sends.
    #[serde(rename = "authKey")]
    pub key: String,
    /// The account.
    pub user: User,
}

/// A Stremio account.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct User {
    /// Its id.
    #[serde(rename = "_id")]
    pub id: String,
    /// The email address it signs in with.
    pub email: String,
}

/// What can go wrong calling the API.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The API could not be reached, or answered with something else.
    Net(net::Error),
    /// The API refused: its code and message, for example code 1, "Session
    /// does not exist", for a key no longer valid.
    Api {
        /// Stremio's error code.
        code: u64,
        /// Stremio's message.
        message: String,
    },
}

// Public API
impl Api {
    /// A client for the Stremio API at [`API_URL`].
    pub fn new(client: Client) -> Self {
        Self::at(client, API_URL)
    }

    /// A client for an API at `base`, for example a local stand-in; it
    /// ends with `/`.
    pub fn at(client: Client, base: &str) -> Self {
        Self {
            client,
            base: base.to_owned(),
        }
    }

    /// Signs in with an email address and a password.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] when Stremio refuses them, for example code 2
    /// for an unknown email address, and [`Error::Net`] when it cannot be
    /// reached.
    pub fn login(&self, email: &str, password: &str) -> Result<Session> {
        self.call(
            "login",
            &json!({"type": "Login", "email": email, "password": password, "facebook": false}),
        )
    }

    /// Ends the session of `key`.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when Stremio cannot be reached or refuses.
    pub fn logout(&self, key: &str) -> Result {
        self.call::<Value>("logout", &json!({"type": "Logout", "authKey": key}))
            .map(drop)
    }

    /// The account's addons, in the account's order.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when Stremio cannot be reached or refuses, for
    /// example when `key` is no longer valid.
    pub fn addons(&self, key: &str) -> Result<Vec<Addon>> {
        let result: Value = self.call(
            "addonCollectionGet",
            &json!({"type": "AddonCollectionGet", "authKey": key, "update": true}),
        )?;
        Ok(lenient(result.get("addons")))
    }

    /// The account's whole library; items it cannot read are skipped.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when Stremio cannot be reached or refuses.
    pub fn library(&self, key: &str) -> Result<Vec<LibraryItem>> {
        let result: Value = self.call(
            "datastoreGet",
            &json!({"authKey": key, "collection": LIBRARY, "ids": [], "all": true}),
        )?;
        Ok(lenient(Some(&result)))
    }

    /// The library items with these ids, as the account has them now.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when Stremio cannot be reached or refuses.
    pub fn library_items(&self, key: &str, ids: &[&str]) -> Result<Vec<LibraryItem>> {
        let result: Value = self.call(
            "datastoreGet",
            &json!({"authKey": key, "collection": LIBRARY, "ids": ids, "all": false}),
        )?;
        Ok(lenient(Some(&result)))
    }

    /// Writes `items` to the account's library, replacing what it had for
    /// them.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when Stremio cannot be reached or refuses.
    pub fn put_library(&self, key: &str, items: &[LibraryItem]) -> Result {
        self.call::<Value>(
            "datastorePut",
            &json!({"authKey": key, "collection": LIBRARY, "changes": items}),
        )
        .map(drop)
    }
}

impl Error {
    /// Whether the auth key is no longer valid: sign in again.
    pub fn signed_out(&self) -> bool {
        matches!(self, Error::Api { message, .. } if message == "Session does not exist")
    }
}

impl From<net::Error> for Error {
    fn from(e: net::Error) -> Self {
        Error::Net(e)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Net(e) => Some(e),
            Error::Api { .. } => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Net(e) => write!(f, "{e}"),
            Error::Api { message, .. } => write!(f, "{message}"),
        }
    }
}

// Private API
impl Api {
    /// Posts `body` to `method` and reads the result out of the envelope.
    fn call<T: DeserializeOwned>(&self, method: &str, body: &Value) -> Result<T> {
        #[derive(Deserialize)]
        struct ApiError {
            #[serde(default)]
            code: u64,
            #[serde(default)]
            message: String,
        }
        let answer: Value = self.client.post(&format!("{}{method}", self.base), body)?;
        if let Some(error) = answer.get("error").filter(|e| !e.is_null()) {
            let error = ApiError::deserialize(error).map_err(net::Error::Json)?;
            return Err(Error::Api {
                code: error.code,
                message: error.message,
            });
        }
        let result = answer.get("result").cloned().unwrap_or(Value::Null);
        T::deserialize(result).map_err(|e| Error::Net(net::Error::Json(e)))
    }
}
