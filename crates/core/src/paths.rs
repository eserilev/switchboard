//! Where Switchboard keeps its files.
//!
//! `SB_HOME` puts every path under one folder. Tests use it.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Paths {
    /// `~/.switchboard`: state files and pane settings.
    pub home: PathBuf,
    /// `$XDG_RUNTIME_DIR/switchboard`, or `/tmp/sb-<uid>` with no `XDG_RUNTIME_DIR`
    /// (macOS): the socket, the tmux config, nvim sockets.
    pub runtime: PathBuf,
    /// `~/.local/share/switchboard`: the store.
    pub data: PathBuf,
    /// `~/.config/switchboard/config.toml`.
    pub config: PathBuf,
    /// `~/.claude`: Claude Code's own folder.
    pub claude: PathBuf,
}

/// A short folder for sockets when there is no `XDG_RUNTIME_DIR`. macOS has none,
/// and its temp folder is too long: a socket path has at most 104 bytes there.
fn short_runtime() -> PathBuf {
    PathBuf::from(format!("/tmp/sb-{}", uid()))
}

/// The user id, from the owner of the home folder.
fn uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(home_dir()).map(|m| m.uid()).unwrap_or(0)
}

/// Makes the socket folder private (mode 700). The socket controls the agents, so
/// the folder must belong to this user: a folder of another user in `/tmp` is an error.
fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::metadata(dir)?;
    if meta.uid() != uid() {
        return Err(std::io::Error::other(format!(
            "{} belongs to another user",
            dir.display()
        )));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
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
            .map(|d| PathBuf::from(d).join("switchboard"))
            .unwrap_or_else(short_runtime);
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        Paths {
            home: home.join(".switchboard"),
            runtime,
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
        private_dir(&self.runtime)?;
        if let Some(p) = self.config.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(())
    }
}

/// The `PATH` of your login shell. A macOS app that starts from Finder or the Dock
/// gets a short `PATH` with no Homebrew, so `tmux`, `git`, `gh` and `claude` are not
/// found. The shell prints its `PATH` between two marks; anything else that the
/// shell prints is left out. Stops after 5 seconds.
pub fn login_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut child = std::process::Command::new(shell)
        .args(["-l", "-c", "printf '<<SBPATH>>%s<<SBPATH>>' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < std::time::Duration::from_secs(5) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut()?, &mut out).ok()?;
    marked_path(&out)
}

fn marked_path(out: &str) -> Option<String> {
    let rest = out.split_once("<<SBPATH>>")?.1;
    let path = rest.split_once("<<SBPATH>>")?.0;
    (!path.is_empty()).then(|| path.to_owned())
}

/// `path` with the usual tool folders added at the end when they are missing.
pub fn with_tool_dirs(path: &str) -> String {
    let home = home_dir();
    let mut parts: Vec<String> = path.split(':').filter(|p| !p.is_empty()).map(str::to_owned).collect();
    for d in [
        "/opt/homebrew/bin".to_owned(),
        "/usr/local/bin".to_owned(),
        home.join(".local/bin").to_string_lossy().into_owned(),
        home.join(".cargo/bin").to_string_lossy().into_owned(),
    ] {
        if !parts.contains(&d) {
            parts.push(d);
        }
    }
    parts.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_short_socket_folder_fits_a_mac_socket_path() {
        // macOS: at most 104 bytes for the socket path, with nvim sockets the longest.
        let p = short_runtime().join("nvim").join("p123456789.sock");
        assert!(p.as_os_str().len() < 104, "{}", p.display());
    }

    #[test]
    fn the_login_path_is_read_between_the_marks() {
        assert_eq!(marked_path("motd text\n<<SBPATH>>/a:/b<<SBPATH>>more"), Some("/a:/b".into()));
        assert_eq!(marked_path("no marks"), None);
        // This machine's shell gives a PATH.
        assert!(login_path().is_some_and(|p| p.contains("/usr/bin")));
        let p = with_tool_dirs("/usr/bin:/opt/homebrew/bin");
        assert!(p.starts_with("/usr/bin:/opt/homebrew/bin:/usr/local/bin"));
        assert_eq!(p.matches("/opt/homebrew/bin").count(), 1);
    }

    #[test]
    fn the_runtime_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("sb-paths-{}", std::process::id()));
        let p = Paths::under(root.clone());
        p.create_all().unwrap();
        let mode = std::fs::metadata(&p.runtime).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::remove_dir_all(root).unwrap();
    }
}
