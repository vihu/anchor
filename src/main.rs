//! anchor: a native Stremio client that plays through your own mpv.

#![deny(unsafe_code, missing_docs, rustdoc::broken_intra_doc_links)]

// The compiled Slint UI, from the `anchor-ui` crate.
use anchor_ui as ui;

use slint::ComponentHandle;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// The application id: the desktop entry, the icon and the window.
const APP_ID: &str = "io.github.vihu.anchor";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Matches the desktop entry, so docks show anchor's icon for the window.
    slint::set_xdg_app_id(APP_ID)?;
    let app = ui::AppWindow::new()?;
    app.run()?;
    Ok(())
}
