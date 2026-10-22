//! Tuma's engine. Everything that is not UI lives here (HTTP, `{{var}}` templating, storage, importers) so the
//! desktop app, the CLI and the tests all share it. See docs/PLAN.md.

pub mod http;

/// The version of this build, shown in the app.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
