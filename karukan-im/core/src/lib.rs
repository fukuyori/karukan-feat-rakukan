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

/// Build identification: crate version plus the git commit the build came
/// from (`0.1.0+a1fe298`, `-dirty` appended for uncommitted changes,
/// `+unknown` when built outside a git checkout). Embedded at compile time
/// by `build.rs` so a running IME can report exactly which build it is.
pub fn version() -> &'static str {
    concat!(env!("CARGO_PKG_VERSION"), "+", env!("KARUKAN_GIT_DESC"))
}
