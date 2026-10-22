//! Tuma's engine. Everything that is not UI lives here (HTTP, `{{var}}` templating, storage, importers) so the
//! desktop app, the CLI and the tests all share it. See docs/PLAN.md.

pub mod check;
pub mod codegen;
pub mod curl;
pub mod docs;
pub mod expect;
pub mod export;
pub mod graphql;
pub mod grpc;
pub mod history;
pub mod http;
pub mod import;
pub mod jsonc;
pub mod mock;
pub mod oauth2;
pub mod openapi;
pub mod runner;
pub mod runs;
pub mod script;
pub mod secrets;
pub mod settings;
pub mod sse;
pub mod vars;
pub mod workspace;

/// The version of this build, shown in the app.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
