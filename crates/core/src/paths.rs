//! Where Switchboard keeps its files.
//!
//! `SB_HOME` puts every path under one folder. Tests use it.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Paths {
    /// `~/.switchboard`: state files and pane settings.
    pub home: PathBuf,
    /// `$XDG_RUNTIME_DIR/switchboard`: the socket, the tmux config, nvim sockets.
    pub runtime: PathBuf,
    /// `~/.local/share/switchboard`: the store.
    pub data: PathBuf,
    /// `~/.config/switchboard/config.toml`.
    pub config: PathBuf,
    /// `~/.claude`: Claude Code's own folder.
    pub claude: PathBuf,
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into())
}

impl Paths {
    pub fn from_env() -> Paths {
        if let Some(root) = std::env::var_os("SB_HOME").map(PathBuf::from) {
            return Paths::under(root);
        }
        let home = home_dir();
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        Paths {
            home: home.join(".switchboard"),
            runtime: runtime.join("switchboard"),
            data: data.join("switchboard"),
            config: config.join("switchboard/config.toml"),
            claude: home.join(".claude"),
        }
    }

    pub fn under(root: PathBuf) -> Paths {
        Paths {
            home: root.join("home"),
            runtime: root.join("run"),
            data: root.join("data"),
            config: root.join("config.toml"),
            claude: root.join("claude"),
        }
    }

    pub fn sock(&self) -> PathBuf {
        std::env::var_os("SB_SOCK")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.runtime.join("sock"))
    }
    pub fn state_dir(&self) -> PathBuf {
        self.home.join("state")
    }
    pub fn settings_dir(&self) -> PathBuf {
        self.home.join("settings")
    }
    pub fn store(&self) -> PathBuf {
        self.data.join("store.db")
    }
    pub fn tmux_conf(&self) -> PathBuf {
        self.runtime.join("tmux.conf")
    }
    pub fn nvim_dir(&self) -> PathBuf {
        self.runtime.join("nvim")
    }

    pub fn create_all(&self) -> std::io::Result<()> {
        for d in [
            &self.home,
            &self.runtime,
            &self.data,
            &self.state_dir(),
            &self.settings_dir(),
            &self.nvim_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        if let Some(p) = self.config.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(())
    }
}
