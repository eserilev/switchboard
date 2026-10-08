//! Switchboard core: everything below the window.

pub mod config;
pub mod connections;
pub mod control;
pub mod diff;
pub mod hooks;
pub mod hub;
pub mod hub_review;
pub mod lamp;
pub mod memory;
pub mod nvim;
pub mod paths;
pub mod proto;
pub mod repos;
pub mod review;
pub mod store;
pub mod tmux;

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Replaces a leading `~` with the home folder.
pub fn expand(path: &str) -> std::path::PathBuf {
    match path.strip_prefix('~') {
        Some(rest) => paths::home_dir().join(rest.trim_start_matches('/')),
        None => path.into(),
    }
}
