//! The `git` tool of `sb mcp` for the guide agent (SPEC 11.4).
//!
//! The guide agent only reads. A Bash rule such as `Bash(git diff:*)` cannot keep it
//! read-only: `git diff --output=<file>` writes a file, and `--ext-diff`,
//! `--textconv` or `--no-index` run programs or read files outside the tree. So the
//! agent has no Bash. It gets this tool: a subcommand from a fixed list, and args
//! that must each pass a list of safe options. No shell runs.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// The subcommands the tool runs.
pub const SUBCOMMANDS: [&str; 6] = ["diff", "log", "show", "blame", "merge-base", "rev-parse"];

/// The most output the tool returns.
pub const MAX_OUTPUT: usize = 256 * 1024;

/// The longest one git call may take.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Why an arg was refused, in words.
fn why(code: u8, arg: &str) -> String {
    match code {
        git_argcheck::EMPTY => "an empty arg".into(),
        git_argcheck::CONTROL => format!("{arg:?}: no control characters"),
        git_argcheck::PATH => format!("{arg}: use a path inside the review worktree"),
        _ => format!("{arg}: this option is not allowed"),
    }
}

/// Checks a git call. Returns the args to pass after the subcommand, with the
/// options that turn off external diff and textconv programs first. The check is
/// the verified kernel `git-argcheck` (SPEC 24.1).
pub fn check(sub: &str, args: &[String]) -> Result<Vec<String>, String> {
    let bytes: Vec<Vec<u8>> = args.iter().map(|a| a.as_bytes().to_vec()).collect();
    match git_argcheck::check(sub.as_bytes(), &bytes) {
        Ok(out) => Ok(out.into_iter().map(|a| String::from_utf8_lossy(&a).into_owned()).collect()),
        Err(r) if r.why == git_argcheck::BAD_SUB => Err(format!(
            "git {sub} is not allowed. Use one of: {}.",
            SUBCOMMANDS.join(", ")
        )),
        Err(r) => {
            let arg = args.get(r.at).map(String::as_str).unwrap_or("");
            Err(format!("git {sub}: {}. Use plain revs, ranges, paths and read options.", why(r.why, arg)))
        }
    }
}

/// Runs a checked git call in `dir`. No shell, no pager, no system config, no
/// external diff. The output is cut at `MAX_OUTPUT`.
pub fn run(dir: &Path, sub: &str, args: &[String]) -> Result<String, String> {
    let args = check(sub, args)?;
    let mut cmd = Command::new("git");
    cmd.args(["--no-pager", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "color.ui=false", "-c", "core.pager=cat", "-C"])
        .arg(dir)
        .arg(sub)
        .args(&args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for v in [
        "GIT_EXTERNAL_DIFF",
        "GIT_DIFF_OPTS",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_GLOBAL",
        "GIT_EXEC_PATH",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_ASKPASS",
    ] {
        cmd.env_remove(v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
    let pid = child.id() as i32;
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(TIMEOUT).is_err() {
            // The child is not reaped before `done` is sent, so the pid is still its.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    });
    let mut err = child.stderr.take().expect("piped");
    let err_text = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut err).take(8 * 1024).read_to_end(&mut buf);
        // Drain the rest, so a full pipe does not block git.
        let _ = std::io::copy(&mut err, &mut std::io::sink());
        String::from_utf8_lossy(&buf).trim().to_owned()
    });
    let mut out = Vec::new();
    let stdout = child.stdout.take().expect("piped");
    let _ = stdout.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut out);
    let cut = out.len() > MAX_OUTPUT;
    if cut {
        let _ = child.kill();
        out.truncate(MAX_OUTPUT);
    }
    let status = child.wait();
    let _ = done_tx.send(());
    let _ = watchdog.join();
    let err = err_text.join().unwrap_or_default();
    let status = status.map_err(|e| format!("git: {e}"))?;
    let mut text = String::from_utf8_lossy(&out).into_owned();
    if cut {
        text += &format!("\n[The output was cut at {} KB. Ask for less: a path, a range, or -U0.]", MAX_OUTPUT / 1024);
        return Ok(text);
    }
    if !status.success() {
        let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into());
        return Err(format!("git exited with {code}. {err}\n{text}").trim().to_owned());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn normal_calls_pass() {
        for (sub, args) in [
            ("diff", vec!["-U0", "abc123", "HEAD"]),
            ("diff", vec!["--stat", "main...HEAD", "--", "src/a.rs"]),
            ("diff", vec!["--unified=3", "--name-only", "HEAD~1..HEAD"]),
            ("diff", vec!["-M50%", "--diff-filter=AM", "a..b", "--", "x/y z.rs"]),
            ("log", vec!["--oneline", "-n20", "-5", "--format=%H %s", "base..HEAD"]),
            ("log", vec!["-p", "--follow", "--", "src/lib.rs"]),
            ("log", vec!["-L10,20:src/lib.rs", "-Sneedle", "--grep=fix"]),
            ("show", vec!["HEAD:src/lib.rs"]),
            ("show", vec!["--stat", "abc", "-U0"]),
            ("blame", vec!["-L", "10,20", "HEAD", "--", "src/lib.rs"]),
            ("blame", vec!["-L10,+5", "-w", "--porcelain", "src/lib.rs"]),
            ("merge-base", vec!["HEAD", "origin/main"]),
            ("merge-base", vec!["--is-ancestor", "a", "b"]),
            ("rev-parse", vec!["--short", "HEAD"]),
            ("rev-parse", vec!["--verify", "HEAD~2^{commit}"]),
            ("diff", vec!["@{-1}", ":/fix"]),
        ] {
            assert!(check(sub, &s(&args)).is_ok(), "{sub} {args:?}: {:?}", check(sub, &s(&args)));
        }
    }

    #[test]
    fn writes_programs_and_outside_paths_are_refused() {
        for (sub, args) in [
            ("diff", vec!["--output=/tmp/x"]),
            ("diff", vec!["--output", "/tmp/x"]),
            ("diff", vec!["--out=x"]),
            ("log", vec!["--output=x", "HEAD"]),
            ("show", vec!["--output-directory=x"]),
            ("diff", vec!["-ox"]),
            ("diff", vec!["-Ox"]),
            ("diff", vec!["-O", "order"]),
            ("diff", vec!["--ext-diff"]),
            ("diff", vec!["--textconv"]),
            ("diff", vec!["--no-index", "a", "b"]),
            ("log", vec!["-c", "x"]),
            ("diff", vec!["-cfoo=bar"]),
            ("log", vec!["--exec-path=/x"]),
            ("log", vec!["--git-dir=/x"]),
            ("log", vec!["--work-tree=/x"]),
            ("log", vec!["--show-signature"]),
            ("log", vec!["--remerge-diff"]),
            ("blame", vec!["--contents=/etc/passwd", "a"]),
            ("blame", vec!["-Srevs", "a"]),
            ("blame", vec!["--ignore-revs-file=x", "a"]),
            ("diff", vec!["/etc/passwd", "a"]),
            ("diff", vec!["../../x", "a"]),
            ("diff", vec!["a/../../x"]),
            ("diff", vec!["~/.ssh/id_rsa"]),
            ("diff", vec![".."]),
            ("diff", vec![""]),
            ("diff", vec!["--", "--output=x"]),
            ("log", vec!["--grep", "--", "--output=x"]),
            ("diff", vec!["-pw"]),
            ("diff", vec!["-U"]),
            ("diff", vec!["-Uabc"]),
            ("diff", vec!["a\nb"]),
            ("rev-parse", vec!["--git-path=x"]),
            ("rev-parse", vec!["--local-env-vars"]),
            ("merge-base", vec!["--output=x"]),
            ("diff", vec!["--color=always"]),
        ] {
            assert!(check(sub, &s(&args)).is_err(), "{sub} {args:?} passed");
        }
    }

    #[test]
    fn messages_name_the_refused_arg() {
        assert_eq!(
            check("push", &[]).unwrap_err(),
            "git push is not allowed. Use one of: diff, log, show, blame, merge-base, rev-parse."
        );
        let e = |sub: &str, a: &[&str]| check(sub, &s(a)).unwrap_err();
        let tail = ". Use plain revs, ranges, paths and read options.";
        assert_eq!(e("diff", &["HEAD", ""]), format!("git diff: an empty arg{tail}"));
        assert_eq!(e("diff", &["a\nb"]), format!("git diff: \"a\\nb\": no control characters{tail}"));
        assert_eq!(e("diff", &["HEAD", "--output=x"]), format!("git diff: --output=x: this option is not allowed{tail}"));
        assert_eq!(e("log", &["../x"]), format!("git log: ../x: use a path inside the review worktree{tail}"));
    }

    #[test]
    fn the_subcommands_match_the_kernel() {
        assert_eq!(SUBCOMMANDS.join(" ").as_bytes(), git_argcheck::SUBS);
    }

    #[test]
    fn only_read_subcommands_run() {
        for sub in ["push", "commit", "config", "checkout", "apply", "format-patch", "-c", "--output=x", ""] {
            assert!(check(sub, &[]).is_err(), "{sub}");
        }
    }

    #[test]
    fn external_programs_are_turned_off_first() {
        assert_eq!(check("diff", &s(&["HEAD"])).unwrap(), s(&["--no-ext-diff", "--no-textconv", "HEAD"]));
        assert_eq!(check("blame", &s(&["a"])).unwrap(), s(&["--no-textconv", "a"]));
        assert_eq!(check("rev-parse", &s(&["HEAD"])).unwrap(), s(&["HEAD"]));
    }

    #[test]
    fn runs_in_the_tree_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("sb-review-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |a: &[&str]| {
            let st = Command::new("git").arg("-C").arg(&dir).args(a).output().unwrap();
            assert!(st.status.success(), "{a:?}: {}", String::from_utf8_lossy(&st.stderr));
        };
        g(&["init", "-q"]);
        g(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "one"]);
        std::fs::write(dir.join("a.txt"), "x\n").unwrap();
        g(&["add", "a.txt"]);
        g(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "two"]);
        let out = run(&dir, "diff", &s(&["-U0", "HEAD~1", "HEAD"])).unwrap();
        assert!(out.contains("+x"), "{out}");
        let out = run(&dir, "log", &s(&["--format=%s"])).unwrap();
        assert_eq!(out.trim(), "two\none");
        assert!(run(&dir, "rev-parse", &s(&["--verify", "nope"])).is_err());
        assert!(run(&dir, "diff", &s(&["--output=out.txt", "HEAD~1"])).is_err());
        assert!(!dir.join("out.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
