//! The anchor window: the Slint files in `ui/`, compiled to Rust.
//!
//! A crate of its own, so a change to the app's Rust code does not compile
//! the UI again.

slint::include_modules!();
