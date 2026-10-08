//! rust-analyzer memory for each pane (SPEC 15). Linux `/proc` only.

use std::fs;

/// The RSS in bytes of every `rust-analyzer` under `pid`, or `None` when none runs.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_rust_analyzer_under_this_test() {
        assert_eq!(rust_analyzer_rss(std::process::id()), None);
    }

    #[test]
    fn rss_of_self_is_positive() {
        assert!(rss(std::process::id()).unwrap() > 0);
    }

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
