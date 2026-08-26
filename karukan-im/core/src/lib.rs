//! karukan-im: Japanese IME engine shared by the fcitx5 (Linux) and
//! macOS frontends.
//!
//! - fcitx5 C FFI lives in the separate `karukan-fcitx5` crate.
//! - The macOS stdio JSON-RPC server lives in [`server`] and is built
//!   as the `karukan-imserver` binary, bundled inside `karukan-macos`.

pub mod config;
pub mod core;
pub mod server;

pub use core::engine::{EngineAction, EngineResult, InputMethodEngine};
pub use core::keycode::{KeyEvent, KeyModifiers, Keysym};
pub use core::state::InputState;

/// Build identification, embedded at compile time by `build.rs` so a
/// running IME can report exactly which build it is. A build at a release
/// tag reports the tag (`v0.1.0-rakukan.2`), one between releases the
/// distance from the last (`v0.1.0-rakukan.2-3-g1234567`), `-dirty`
/// appended for uncommitted changes; without a reachable tag the form is
/// `0.1.0+a1fe298`, and `0.1.0+unknown` outside a git checkout.
pub fn version() -> &'static str {
    // The version sits in rodata wrapped in a greppable marker with a
    // closing `:` sentinel, so install.sh can read the installed build's
    // version via `strings`: adjacent rodata literals concatenate in that
    // output, and without the sentinel a following string starting with
    // hex chars would be read as part of the commit hash.
    const MARKED: &str = concat!("karukan-version:", env!("KARUKAN_VERSION"), ":");
    &MARKED["karukan-version:".len()..MARKED.len() - 1]
}
