//! `.env` loading lives in rhcs-core so the CLI and the daemon share it.
pub use rhcs_core::env::{load, project_dir};
