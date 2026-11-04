//! One nvim for each worktree (SPEC 10), driven with `nvim --server`.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The socket for a worktree: a hash of its path, so the name is short.
pub fn socket(dir: &Path, tree: &Path) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tree.hash(&mut h);
    dir.join(format!("{:016x}.sock", h.finish()))
}

/// Runs a Vim expression in the nvim on `sock`. Works in every mode.
///
/// An nvim that waits at a "Press ENTER" prompt never answers, so each call
/// gives up after 3 s.
pub fn expr(sock: &Path, expr: &str) -> Result<String, String> {
    let mut child = Command::new("nvim")
        .arg("--server")
        .arg(sock)
        .arg("--remote-expr")
        .arg(expr)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("nvim: {e}"))?;
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "nvim did not answer in 3 s. It waits at a prompt; look at its pane.".into(),
                );
            }
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn alive(sock: &Path) -> bool {
    sock.exists() && expr(sock, "1").map(|r| r == "1").unwrap_or(false)
}

/// Runs `e` in a thread once the nvim on `sock` answers. A new nvim needs a
/// moment before it listens, and an nvim at a "Press ENTER" prompt answers only
/// after you press Enter. Gives up after `wait`. `what` names the call in the log.
pub fn when_ready(sock: PathBuf, e: String, wait: Duration, what: &'static str) {
    std::thread::spawn(move || {
        let end = Instant::now() + wait;
        while Instant::now() < end {
            if alive(&sock) {
                if let Err(err) = expr(&sock, &e) {
                    tracing::warn!(target: "sb::nvim", %err, what, "nvim call failed");
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        tracing::warn!(target: "sb::nvim", what, "nvim did not answer in time: call dropped");
    });
}

/// The argv for a new nvim pane.
pub fn argv(sock: &Path, file: Option<(&str, u32)>) -> Vec<String> {
    let mut a = vec![
        "nvim".to_owned(),
        "--listen".to_owned(),
        sock.to_string_lossy().into_owned(),
    ];
    match file {
        Some((f, line)) => {
            a.push(format!("+{line}"));
            // `--`: a file name that starts with `-` or `+` is still a file.
            a.push("--".into());
            a.push(f.to_owned());
        }
        None => a.push(".".into()),
    }
    a
}

/// The expression that opens a file at a line.
pub fn edit_expr(file: &str, line: u32) -> String {
    format!(
        "execute('edit +{line} ' .. fnameescape('{}'))",
        file.replace('\'', "''")
    )
}

/// The Lua that shows a file next to its old side (SPEC 11.14).
pub const DIFF_LUA: &str = include_str!("lua/sb_diff.lua");

/// The expression that runs `diff` of the Lua file at `lua`: `tree/path` next to
/// `git show base:path`, in diff mode, at `line`. `luaeval` takes the values as a
/// list, so no Lua is quoted inside a Vim string.
pub fn diff_expr(lua: &Path, tree: &Path, base: &str, path: &str, line: u32) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    format!(
        "luaeval('dofile(_A[1]).diff(_A[2], _A[3], _A[4], _A[5])', [{}, {}, {}, {}, {line}])",
        q(&lua.to_string_lossy()),
        q(&tree.to_string_lossy()),
        q(base),
        q(path)
    )
}

/// Reloads every loaded buffer with no unsaved changes. A changed buffer stays as is.
pub const CHECKTIME: &str = "execute('lua for _, b in ipairs(vim.api.nvim_list_bufs()) do if vim.api.nvim_buf_is_loaded(b) and not vim.bo[b].modified then vim.cmd(\"checktime \" .. b) end end')";

/// The number of listed buffers with changes that are not saved.
pub const MODIFIED: &str = "len(getbufinfo({'bufmodified': 1, 'buflisted': 1}))";

pub fn installed() -> bool {
    Command::new("nvim")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_is_stable_and_differs_per_tree() {
        let d = Path::new("/run/sb");
        assert_eq!(socket(d, Path::new("/a")), socket(d, Path::new("/a")));
        assert_ne!(socket(d, Path::new("/a")), socket(d, Path::new("/b")));
    }

    #[test]
    fn argv_ends_options_before_the_file() {
        let s = Path::new("/run/n.sock");
        assert_eq!(
            argv(s, Some(("--cmd=evil", 3))),
            ["nvim", "--listen", "/run/n.sock", "+3", "--", "--cmd=evil"]
        );
        assert_eq!(argv(s, None), ["nvim", "--listen", "/run/n.sock", "."]);
    }

    #[test]
    fn edit_expr_escapes_quotes() {
        assert_eq!(
            edit_expr("it's.rs", 4),
            "execute('edit +4 ' .. fnameescape('it''s.rs'))"
        );
    }

    #[test]
    fn live_nvim_opens_a_file_at_a_line() {
        if !installed() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("sb-nvim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "1\n2\n3\n4\n5\n").unwrap();
        let sock = dir.join("n.sock");
        let mut child = Command::new("nvim")
            .args(["--headless", "--clean", "--listen"])
            .arg(&sock)
            .spawn()
            .unwrap();
        for _ in 0..50 {
            if alive(&sock) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(alive(&sock));
        expr(&sock, &edit_expr(&file.to_string_lossy(), 4)).unwrap();
        assert_eq!(expr(&sock, "line('.')").unwrap(), "4");
        std::fs::write(&file, "changed\n").unwrap();
        expr(&sock, CHECKTIME).unwrap();
        assert_eq!(expr(&sock, "getline(1)").unwrap(), "changed");
        let _ = expr(&sock, "execute('qa!')");
        let _ = child.wait();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn live_nvim_shows_the_old_side_in_diff_mode() {
        if !installed() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("sb-nvim-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tree = dir.join("it's tree");
        std::fs::create_dir_all(&tree).unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(&tree).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
            assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
            String::from_utf8_lossy(&ok.stdout).trim().to_owned()
        };
        git(&["init", "-q"]);
        std::fs::write(tree.join("f.txt"), "1\n2\n3\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "one"]);
        let base = git(&["rev-parse", "HEAD"]);
        std::fs::write(tree.join("f.txt"), "1\ntwo\n3\n4\n").unwrap();
        std::fs::write(tree.join("new.txt"), "n\n").unwrap();
        let lua = dir.join("sb_diff.lua");
        std::fs::write(&lua, DIFF_LUA).unwrap();
        let sock = dir.join("n.sock");
        // Killed on a panic too, and with no stdout: a live nvim holds no pipe open.
        struct Kill(std::process::Child);
        impl Drop for Kill {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let _child = Kill(Command::new("nvim").args(["--headless", "--clean", "--listen"]).arg(&sock).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        for _ in 0..50 {
            if alive(&sock) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let diff_wins = "len(filter(range(1, winnr('$')), 'getwinvar(v:val, \"&diff\")'))";
        expr(&sock, &diff_expr(&lua, &tree, &base, "f.txt", 3)).unwrap();
        assert_eq!(expr(&sock, "winnr('$')").unwrap(), "2");
        assert_eq!(expr(&sock, diff_wins).unwrap(), "2");
        // The cursor is on the file itself, at the line, and the old side is the base.
        assert_eq!(expr(&sock, "expand('%:t') .. ':' .. line('.')").unwrap(), "f.txt:3");
        assert_eq!(expr(&sock, "string(getbufline(winbufnr(1), 1, '$'))").unwrap(), "['1', '2', '3']");
        // A second call does not pile up splits; a new file has an empty old side.
        expr(&sock, &diff_expr(&lua, &tree, &base, "new.txt", 1)).unwrap();
        assert_eq!(expr(&sock, "winnr('$')").unwrap(), "2");
        assert_eq!(expr(&sock, "string(getbufline(winbufnr(1), 1, '$'))").unwrap(), "['']");
        assert_eq!(expr(&sock, "expand('%:t')").unwrap(), "new.txt");
        let _ = std::fs::remove_dir_all(dir);
    }
}
