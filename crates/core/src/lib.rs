//! Switchboard core: everything below the window.
#![recursion_limit = "256"]

pub mod cleanup;
pub mod base;
pub mod chat;
pub mod config;
pub mod connections;
pub mod control;
pub mod coverage;
pub mod diff;
pub mod dispatch;
pub mod gh;
pub mod handover;
pub mod hooks;
pub mod hub;
pub mod hub_handover;
pub mod hub_mail;
pub mod hub_review;
pub mod hub_show;
pub mod hub_sync;
pub mod hub_tasks;
pub mod hub_today;
pub mod lamp;
pub mod mail;
pub mod memory;
pub mod mentions;
pub mod nvim;
pub mod paths;
pub mod post;
pub mod proto;
pub mod remote;
pub mod repo_context;
pub mod repos;
pub mod pr_work;
pub mod review;
pub mod review_git;
pub mod sandbox;
pub mod store;
pub mod take;
pub mod tasks;
pub mod tmux;
pub mod today;
pub mod today_issues;
pub mod today_notes;

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
