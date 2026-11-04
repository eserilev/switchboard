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

/// Diff options of `diff`, `log` and `show` with no value.
const DIFF_FLAGS: &[&str] = &[
    "-p", "--patch", "-s", "--no-patch", "--stat", "--numstat", "--shortstat",
    "--name-only", "--name-status", "--summary", "--raw", "--patch-with-stat",
    "--patch-with-raw", "--compact-summary", "--dirstat", "--cumulative", "-M",
    "--find-renames", "-C", "--find-copies", "--find-copies-harder", "-D",
    "--irreversible-delete", "--no-renames", "-w", "--ignore-all-space", "-b",
    "--ignore-space-change", "--ignore-space-at-eol", "--ignore-blank-lines",
    "--ignore-cr-at-eol", "--word-diff", "--color-words", "--no-color", "--relative",
    "--no-relative", "-W", "--function-context", "--histogram", "--patience",
    "--minimal", "--abbrev", "--no-abbrev", "--full-index", "-R", "--no-prefix",
    "-a", "--text",
];

/// Diff options of `diff`, `log` and `show` in the `--name=value` form.
const DIFF_VALUES: &[&str] = &[
    "--unified", "--stat", "--dirstat", "--find-renames", "--find-copies",
    "--word-diff", "--word-diff-regex", "--color-words", "--relative",
    "--diff-algorithm", "--diff-filter", "--abbrev", "--src-prefix", "--dst-prefix",
    "--inter-hunk-context", "--anchored",
];

/// Options of `git diff` alone.
const DIFF_ONLY: &[&str] = &["--cached", "--staged", "--merge-base", "--exit-code", "--quiet", "--check"];

/// Log options of `log` and `show` with no value.
const LOG_FLAGS: &[&str] = &[
    "--oneline", "--graph", "--decorate", "--no-decorate", "--first-parent",
    "--no-merges", "--merges", "--reverse", "--follow", "--all", "--parents",
    "--children", "--topo-order", "--date-order", "--author-date-order",
    "--ancestry-path", "--left-right", "--cherry-pick", "--cherry-mark", "--cherry",
    "--boundary", "--full-history", "--simplify-merges", "--simplify-by-decoration",
    "--dense", "--sparse", "--pickaxe-all", "--pickaxe-regex", "-i",
    "--regexp-ignore-case", "-E", "--extended-regexp", "-F", "--fixed-strings",
    "--all-match", "--invert-grep", "-m", "--cc", "--no-walk", "--source",
    "--abbrev-commit", "--no-abbrev-commit", "--pretty", "-L", "-n", "--quiet",
];

/// Log options of `log` and `show` in the `--name=value` form.
const LOG_VALUES: &[&str] = &[
    "--max-count", "--skip", "--since", "--after", "--until", "--before", "--author",
    "--committer", "--grep", "--format", "--pretty", "--date", "--decorate",
    "--diff-merges", "--max-parents", "--min-parents", "--no-walk",
];

const BLAME_FLAGS: &[&str] = &[
    "-w", "-M", "-C", "-s", "-e", "--show-email", "-l", "-t", "-f", "--show-name",
    "-n", "--show-number", "-p", "--porcelain", "--line-porcelain", "--incremental",
    "--root", "--show-stats", "-b", "--first-parent", "--reverse", "--abbrev", "-L",
];

const BLAME_VALUES: &[&str] = &["--date", "--abbrev"];

const MERGE_BASE_FLAGS: &[&str] = &["--is-ancestor", "--fork-point", "--all", "-a", "--octopus", "--independent"];

const REV_PARSE_FLAGS: &[&str] = &[
    "--verify", "--short", "-q", "--quiet", "--abbrev-ref", "--symbolic",
    "--symbolic-full-name", "--show-toplevel", "--show-prefix", "--is-inside-work-tree",
];

const REV_PARSE_VALUES: &[&str] = &["--short", "--abbrev-ref"];

/// True when every char of `s` is a digit, and `s` is not empty.
fn digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// A short option with its value glued on, such as `-U0` or `-n5`.
fn short_with_value(sub: &str, arg: &str) -> bool {
    let rest = &arg[1..];
    let mut chars = rest.chars();
    let Some(flag) = chars.next() else {
        return false;
    };
    let value = chars.as_str();
    if value.is_empty() {
        return false;
    }
    let diffs = matches!(sub, "diff" | "log" | "show");
    let logs = matches!(sub, "log" | "show");
    let blame = sub == "blame";
    match flag {
        'U' => diffs && digits(value),
        // A rename or copy score: `-M50` or `-M50%`.
        'M' | 'C' => (diffs || blame) && digits(value.trim_end_matches('%')),
        'l' => diffs && digits(value),
        // The pickaxe: a string or a regex to look for. In blame, `-S` reads a file.
        'S' | 'G' => diffs,
        'L' => logs || blame,
        'n' => logs && digits(value),
        _ => false,
    }
}

/// Why `arg` may not go to `git <sub>`, or `None` when it may.
fn refuse(sub: &str, arg: &str) -> Option<String> {
    if arg.is_empty() {
        return Some("an empty arg".into());
    }
    if arg.contains('\0') || arg.contains('\n') {
        return Some(format!("{arg:?}: no control characters"));
    }
    if arg == "--" {
        return None;
    }
    if let Some(name) = arg.strip_prefix("--") {
        let (name, value) = match name.split_once('=') {
            Some((n, v)) => (format!("--{n}"), Some(v)),
            None => (arg.to_owned(), None),
        };
        let ok = match (sub, value) {
            ("diff", None) => DIFF_FLAGS.contains(&name.as_str()) || DIFF_ONLY.contains(&name.as_str()),
            ("diff", Some(_)) => DIFF_VALUES.contains(&name.as_str()),
            ("log" | "show", None) => DIFF_FLAGS.contains(&name.as_str()) || LOG_FLAGS.contains(&name.as_str()),
            ("log" | "show", Some(_)) => DIFF_VALUES.contains(&name.as_str()) || LOG_VALUES.contains(&name.as_str()),
            ("blame", None) => BLAME_FLAGS.contains(&name.as_str()),
            ("blame", Some(_)) => BLAME_VALUES.contains(&name.as_str()),
            ("merge-base", None) => MERGE_BASE_FLAGS.contains(&name.as_str()),
            ("rev-parse", None) => REV_PARSE_FLAGS.contains(&name.as_str()),
            ("rev-parse", Some(_)) => REV_PARSE_VALUES.contains(&name.as_str()),
            _ => false,
        };
        return if ok { None } else { Some(format!("{arg}: this option is not allowed")) };
    }
    if let Some(short) = arg.strip_prefix('-') {
        let ok = match sub {
            "diff" => DIFF_FLAGS.contains(&arg),
            "log" | "show" => {
                DIFF_FLAGS.contains(&arg) || LOG_FLAGS.contains(&arg) || (sub == "log" && digits(short))
            }
            "blame" => BLAME_FLAGS.contains(&arg),
            "merge-base" => MERGE_BASE_FLAGS.contains(&arg),
            "rev-parse" => REV_PARSE_FLAGS.contains(&arg),
            _ => false,
        } || short_with_value(sub, arg);
        return if ok { None } else { Some(format!("{arg}: this option is not allowed")) };
    }
    // A rev, a range or a path. It must stay in the review worktree: git diff with
    // a path outside the tree compares files on disk, as with `--no-index`.
    if arg.starts_with('/') || arg.starts_with('~') {
        return Some(format!("{arg}: use a path inside the review worktree"));
    }
    if arg == ".." || arg.starts_with("../") || arg.contains("/../") || arg.ends_with("/..") {
        return Some(format!("{arg}: use a path inside the review worktree"));
    }
    None
}

/// Checks a git call. Returns the args to pass after the subcommand, with the
/// options that turn off external diff and textconv programs first.
pub fn check(sub: &str, args: &[String]) -> Result<Vec<String>, String> {
    if !SUBCOMMANDS.contains(&sub) {
        return Err(format!(
            "git {sub} is not allowed. Use one of: {}.",
            SUBCOMMANDS.join(", ")
        ));
    }
    let mut out: Vec<String> = match sub {
        "diff" | "log" | "show" => vec!["--no-ext-diff".into(), "--no-textconv".into()],
        "blame" => vec!["--no-textconv".into()],
        _ => vec![],
    };
    for a in args {
        if let Some(why) = refuse(sub, a) {
            return Err(format!("git {sub}: {why}. Use plain revs, ranges, paths and read options."));
        }
        out.push(a.clone());
    }
    Ok(out)
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
