//! rust-analyzer memory for each pane (SPEC 15). Linux reads `/proc`; macOS, which
//! has no `/proc`, reads one `ps` table.

#[cfg(target_os = "linux")]
use std::fs;

/// The RSS in bytes of every `rust-analyzer` under `pid`, or `None` when none runs.
#[cfg(not(target_os = "linux"))]
pub fn rust_analyzer_rss(pid: u32) -> Option<u64> {
    ps::rust_analyzer_rss(&ps::table(), pid)
}

/// The RSS in bytes of every `rust-analyzer` under `pid`, or `None` when none runs.
#[cfg(target_os = "linux")]
pub fn rust_analyzer_rss(pid: u32) -> Option<u64> {
    let mut total = 0;
    let mut found = false;
    let mut stack = vec![pid];
    let mut seen = 0;
    while let Some(p) = stack.pop() {
        seen += 1;
        if seen > 4096 {
            break;
        }
        if fs::read_to_string(format!("/proc/{p}/comm"))
            .map(|c| c.trim() == "rust-analyzer")
            .unwrap_or(false)
        {
            found = true;
            total += rss(p).unwrap_or(0);
        }
        stack.extend(children(p));
    }
    found.then_some(total)
}

#[cfg(target_os = "linux")]
fn children(pid: u32) -> Vec<u32> {
    let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) else {
        return vec![];
    };
    tasks
        .flatten()
        .filter_map(|t| fs::read_to_string(t.path().join("children")).ok())
        .flat_map(|s| {
            s.split_whitespace()
                .filter_map(|n| n.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn rss(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kb: u64 = status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(kb * 1024)
}

/// The process table from `ps`: macOS and Linux both have these columns.
#[cfg_attr(target_os = "linux", allow(dead_code))]
mod ps {
    /// One process: pid, parent pid, RSS in KiB, and the program name.
    pub struct Proc {
        pub pid: u32,
        pub ppid: u32,
        pub rss_kb: u64,
        pub name: String,
    }

    pub fn table() -> Vec<Proc> {
        let out = std::process::Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,rss=,comm="])
            .stdin(std::process::Stdio::null())
            .output();
        match out {
            Ok(o) => parse(&String::from_utf8_lossy(&o.stdout)),
            Err(_) => vec![],
        }
    }

    /// Lines of `pid ppid rss command`. On macOS the command is a full path.
    pub fn parse(text: &str) -> Vec<Proc> {
        text.lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let pid = it.next()?.parse().ok()?;
                let ppid = it.next()?.parse().ok()?;
                let rss_kb = it.next()?.parse().ok()?;
                let cmd = it.collect::<Vec<_>>().join(" ");
                let name = cmd.rsplit('/').next().unwrap_or("").to_owned();
                Some(Proc { pid, ppid, rss_kb, name })
            })
            .collect()
    }

    pub fn rust_analyzer_rss(table: &[Proc], pid: u32) -> Option<u64> {
        let mut total = 0;
        let mut found = false;
        let mut stack = vec![pid];
        let mut seen = 0;
        while let Some(p) = stack.pop() {
            seen += 1;
            if seen > 4096 {
                break;
            }
            if let Some(me) = table.iter().find(|x| x.pid == p) {
                if me.name == "rust-analyzer" {
                    found = true;
                    total += me.rss_kb * 1024;
                }
            }
            stack.extend(table.iter().filter(|x| x.ppid == p && x.pid != p).map(|x| x.pid));
        }
        found.then_some(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ps_table_finds_rust_analyzer_under_a_pane() {
        let t = ps::parse(
            "  10     1  500 /bin/zsh\n  11    10 2048 /Users/me/.cargo/bin/rust-analyzer\n  12    11 1024 rust-analyzer\n  13     1 9999 rust-analyzer\n",
        );
        assert_eq!(ps::rust_analyzer_rss(&t, 10), Some(3072 * 1024));
        assert_eq!(ps::rust_analyzer_rss(&t, 99), None);
        // The real table has this process in it.
        assert!(ps::table().iter().any(|p| p.pid == std::process::id()));
    }

    #[test]
    fn no_rust_analyzer_under_this_test() {
        assert_eq!(rust_analyzer_rss(std::process::id()), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rss_of_self_is_positive() {
        assert!(rss(std::process::id()).unwrap() > 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn children_finds_a_child() {
        let mut c = std::process::Command::new("sleep")
            .arg("2")
            .spawn()
            .unwrap();
        assert!(children(std::process::id()).contains(&c.id()));
        c.kill().unwrap();
        c.wait().unwrap();
    }
}
