//! anchor: a native Stremio client that plays through your own mpv.
//!
//! Opens the saved Stremio account (or the sign-in screen), browses its
//! addons, and plays streams in the user's own mpv.

#![deny(unsafe_code, missing_docs, rustdoc::broken_intra_doc_links)]

mod art;
mod session;
#[cfg(feature = "snapshots")]
mod snapshots;

// The compiled Slint UI, from the `anchor-ui` crate.
use anchor_ui as ui;

use anchor::addon::Addons;
use anchor::api::Api;
use anchor::net::Client;
use anchor::store::Paths;
use slint::ComponentHandle;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// The application id: the desktop entry, the icon and the window.
const APP_ID: &str = "io.github.vihu.anchor";
/// Points anchor at another Stremio API, for example `tools/fake-stremio.py`.
const API_VARIABLE: &str = "ANCHOR_API_URL";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "snapshots")]
    if let Some(dir) = snapshots::requested() {
        snapshots::install(dir)?;
    }
    // Matches the desktop entry, so docks show anchor's icon for the window.
    slint::set_xdg_app_id(APP_ID)?;
    let client = Client::new();
    let api = match std::env::var(API_VARIABLE) {
        Ok(url) if !url.is_empty() => Api::at(client.clone(), &url),
        _ => Api::new(client.clone()),
    };
    let app = ui::AppWindow::new()?;
    #[cfg(feature = "snapshots")]
    snapshots::attach(&app);
    session::start(&app, Paths::system()?, api, Addons::new(client));
    app.run()?;
    session::finish();
    Ok(())
}
