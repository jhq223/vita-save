//! Save discovery, verified snapshots, and WebDAV storage.
pub mod app;
pub mod backup;
pub mod cloud;
pub mod config;
pub mod job;
pub mod platform;
pub mod saves;
pub mod text_edit;
pub mod ui;
pub const APP_NAME: &str = "Vita Save";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
