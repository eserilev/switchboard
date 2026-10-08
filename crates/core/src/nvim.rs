//! One nvim for each worktree (SPEC 10), driven with `nvim --server`.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The socket for a worktree: a hash of its path, so the name is short.
pub fn socket(dir: &Path, tree: &Path) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tree.hash(&mut h);
    dir.join(format!("{:016x}.sock", h.finish()))
}

/// Runs a Vim expression in the nvim on `sock`. Works in every mode.
pub fn expr(sock: &Path, expr: &str) -> Result<String, String> {
    let out = Command::new("nvim")
        .arg("--server")
        .arg(sock)
        .arg("--remote-expr")
        .arg(expr)
        .output()
        .map_err(|e| format!("nvim: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn alive(sock: &Path) -> bool {
    sock.exists() && expr(sock, "1").map(|r| r == "1").unwrap_or(false)
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

/// Reloads every loaded buffer with no unsaved changes. A changed buffer stays as is.
pub const CHECKTIME: &str = "execute('lua for _, b in ipairs(vim.api.nvim_list_bufs()) do if vim.api.nvim_buf_is_loaded(b) and not vim.bo[b].modified then vim.cmd(\"checktime \" .. b) end end')";

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
}
