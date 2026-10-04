//! anchor: a native Stremio client that plays through your own mpv.
//!
//! The library holds everything that does not need a window. The `anchor`
//! binary wires it to Slint.

#![deny(unsafe_code, missing_docs, rustdoc::broken_intra_doc_links)]

pub mod addon;
pub mod net;
pub mod player;
pub mod settings;
pub mod store;
