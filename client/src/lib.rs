pub mod manifest;
pub mod paths;
pub mod ready;
pub mod static_server;
pub mod update;

#[cfg(windows)]
pub mod windows;

slint::include_modules!();

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASE_MANIFEST_URL: &str =
    "https://github.com/ztb0y/tdl-workspace/releases/latest/download/release-manifest.json";
