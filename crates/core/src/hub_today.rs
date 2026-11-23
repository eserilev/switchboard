//! Today in the hub (SPEC 39): the refresh thread, Done and Later, and the list
//! for the window, the pane tools and `sb today`. The rules live in `today`.

use crate::gh::Gh;
use crate::hub::Hub;
use crate::now;
use crate::today::{self, Break, Ctx, Data, Item, Seen, CHANGED_KEY, EVERYTHING};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

type Res<T> = Result<T, String>;

/// What the last refresh got.
#[derive(Default)]
pub struct TodayState {
    data: Option<Data>,
    breaks: Vec<Break>,
    me: Option<String>,
    fetched: Option<u64>,
    error: Option<String>,
    running: bool,
    /// Set when Today was opened once: from then on the tick refreshes it.
    opened: bool,
}

/// `YYYY-MM-DD` of a time, for a GitHub search.
pub fn ymd(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    // The civil-from-days rule.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// The searches of one refresh (SPEC 39.6): the open buckets, the area searches
/// and the closed one. One fetch serves every job: a job is a filter on it.
pub fn searches(me: &str, cfg: &today::TodayCfg, now: u64) -> (Vec<String>, Vec<String>, String) {
    let d = |n: u64| ymd(now.saturating_sub(n * 86400));
    let open = vec![
        format!("is:pr is:open archived:false author:{me}"),
        format!("is:pr is:open archived:false review-requested:{me}"),
        format!("is:pr is:open archived:false reviewed-by:{me} -author:{me} updated:>={}", d(30)),
        format!("is:pr is:open archived:false mentions:{me} -author:{me} updated:>={}", d(14)),
    ];
    let area = cfg.area_repos().iter().map(|r| format!("repo:{r} is:pr is:open -author:{me} updated:>={}", d(30))).collect();
    (open, area, format!("is:pr author:{me} is:closed closed:>={}", d(3)))
}

/// One search. A failed answer (a GitHub timeout, a cut-off reply) tries once
/// more, for fewer PRs. A login or query error does not.
fn search(gh: &dyn Gh, q: &str) -> Res<Vec<today::Pr>> {
    let got = match gh.graphql(&today::search_query(), json!({ "q": q })) {
        Err(e) if !e.contains("not logged in") && !e.starts_with("GitHub:") => gh.graphql(&today::search_query_of(10), json!({ "q": q })),
        other => other,
    };
    match got? {
        Some(data) => Ok(today::parse_search(&data)),
        None => Ok(vec![]),
    }
}

/// One refresh (SPEC 39.6): every search side by side, then the merges into the
/// repos of your conflicting PRs. Some searches can fail; the rest still count.
pub fn fetch(gh: &dyn Gh, me: &str, cfg: &today::TodayCfg, t: u64) -> (Option<Data>, Option<String>) {
    let (open_q, area_q, closed_q) = searches(me, cfg, t);
    let mut all: Vec<String> = open_q.clone();
    all.extend(area_q.iter().cloned());
    all.push(closed_q);
    let results: Vec<Res<Vec<today::Pr>>> = std::thread::scope(|s| {
        let hs: Vec<_> = all.iter().map(|q| s.spawn(move || search(gh, q))).collect();
        hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a search stopped".into()))).collect()
    });
    let mut errors = vec![];
    let mut data = Data::default();
    let mut got = 0;
    for (i, r) in results.into_iter().enumerate() {
        match r {
            Ok(prs) => {
                got += 1;
                if i < open_q.len() {
                    data.open.extend(prs);
                } else if i < open_q.len() + area_q.len() {
                    data.area.extend(prs);
                } else {
                    data.closed.extend(prs);
                }
            }
            Err(e) => errors.push(e),
        }
    }
    if got == 0 {
        return (None, errors.into_iter().next());
    }
    let mut repos: Vec<String> = data.open.iter().filter(|p| p.author.eq_ignore_ascii_case(me) && p.mergeable == "CONFLICTING").map(|p| p.repo.clone()).collect();
    repos.sort();
    repos.dedup();
    for r in repos.iter().take(5) {
        match search(gh, &format!("repo:{r} is:pr is:merged merged:>={}", ymd(t.saturating_sub(7 * 86400)))) {
            Ok(prs) => data.merged.extend(prs),
            Err(e) => errors.push(e),
        }
    }
    let err = (!errors.is_empty()).then(|| format!("Some of GitHub did not load: {}", errors[0]));
    (Some(data), err)
}

impl Hub {
    /// One refresh: every search, side by side. Some can fail; the rest still count.
    fn today_fetch(&self) -> (Option<Data>, Option<String>, Option<String>) {
        let me = match self.today_me() {
            Ok(m) => m,
            Err(e) => return (None, None, Some(e)),
        };
        let gh = self.gh.read().unwrap().clone();
        let (data, err) = fetch(gh.as_ref(), &me, &self.config().today, now());
        (data, Some(me), err)
    }

    /// Your login: the config's, the last one, or the one of `gh`.
    fn today_me(&self) -> Res<String> {
        if let Some(u) = self.config().today.user.filter(|u| !u.is_empty()) {
            return Ok(u);
        }
        let known = self.today.lock().unwrap().me.clone();
        if let Some(me) = known {
            return Ok(me);
        }
        let gh = self.gh.read().unwrap().clone();
        gh.login()
    }

    /// The conflicts that are new since the last refresh (SPEC 39.4, upstream_breaks).
    fn today_breaks(&self, data: &Data, me: &str) -> Vec<Break> {
        let t = now();
        let store = self.store.lock().unwrap();
        let old = store.today_prs().unwrap_or_default();
        let mut out = vec![];
        for p in data.open.iter().filter(|p| p.author.eq_ignore_ascii_case(me)) {
            let k = (p.repo.to_lowercase(), p.number);
            let prev = old.get(&k);
            // UNKNOWN: GitHub has not worked it out yet. The next refresh will.
            if p.mergeable == "UNKNOWN" || p.mergeable.is_empty() {
                continue;
            }
            let (broke_at, cause) = match (p.mergeable.as_str(), prev) {
                ("CONFLICTING", Some((m, _, _))) if m == "MERGEABLE" => (Some(t), today::conflict_cause(p, &data.merged)),
                ("CONFLICTING", Some((_, b, c))) => (*b, *c),
                _ => (None, None),
            };
            let _ = store.today_pr_set(&p.repo, p.number, &p.mergeable, broke_at, cause);
            if let Some(at) = broke_at {
                out.push(Break { repo: p.repo.clone(), number: p.number, at, cause });
            }
        }
        out
    }

    /// Starts a refresh on its own thread, unless one runs.
    pub(crate) fn today_refresh(&self) {
        {
            let mut st = self.today.lock().unwrap();
            if st.running {
                return;
            }
            st.running = true;
            st.opened = true;
        }
        self.sink.emit("today", json!({ "refreshing": true }));
        let me = self.me.clone();
        let spawned = std::thread::Builder::new().name("sb-today".into()).spawn(move || {
            let Some(hub) = me.upgrade() else { return };
            let t0 = std::time::Instant::now();
            let (data, login, err) = hub.today_fetch();
            let breaks = match (&data, &login) {
                (Some(d), Some(m)) => hub.today_breaks(d, m),
                _ => vec![],
            };
            tracing::info!(target: "sb::today", ms = t0.elapsed().as_millis() as u64, ok = data.is_some(), err = ?err, "refresh");
            {
                let mut st = hub.today.lock().unwrap();
                st.running = false;
                st.error = err;
                if login.is_some() {
                    st.me = login;
                }
                if let Some(d) = data {
                    st.data = Some(d);
                    st.breaks = breaks;
                    st.fetched = Some(now());
                }
            }
            hub.sink.emit("today", json!({ "refreshing": false }));
        });
        if spawned.is_err() {
            self.today.lock().unwrap().running = false;
        }
    }

    /// From the tick: a refresh when Today was opened and the list is old.
    pub(crate) fn today_tick(&self) {
        let mins = self.config().today.refresh_minutes;
        let (opened, fetched) = {
            let st = self.today.lock().unwrap();
            (st.opened, st.fetched)
        };
        if opened && mins > 0 && fetched.is_none_or(|f| now().saturating_sub(f) >= mins * 60) {
            self.today_refresh();
        }
    }

    /// Every item of the last refresh, before Done and Later.
    fn today_items(&self) -> (Vec<Item>, Value) {
        let cfg = self.config().today;
        let (data, breaks, me, meta) = {
            let st = self.today.lock().unwrap();
            let meta = json!({
                "login": st.me, "updated": st.fetched, "refreshing": st.running,
                "error": st.error, "loaded": st.data.is_some(),
            });
            (st.data.clone(), st.breaks.clone(), st.me.clone(), meta)
        };
        let items = match (data, me) {
            (Some(d), Some(me)) => today::items(&d, &breaks, &Ctx { cfg: &cfg, me: me.to_lowercase(), now: now() }),
            _ => vec![],
        };
        (items, meta)
    }

    fn today_view_of(&self, job: &str) -> (today::View, Value) {
        let cfg = self.config().today;
        let (items, meta) = self.today_items();
        let (seen, looked) = {
            let store = self.store.lock().unwrap();
            let seen = store.today_seen().unwrap_or_default();
            let looked = store.get(&format!("today.looked:{job}")).ok().flatten().and_then(|v| v.parse().ok());
            (seen, looked)
        };
        (today::view(&items, &cfg, job, &seen, looked, now()), meta)
    }

    /// `today(job)`: the list of one job. Starts a refresh when it is old.
    pub fn today(&self, job: Option<&str>) -> Res<Value> {
        let job = job.unwrap_or(EVERYTHING);
        let mins = self.config().today.refresh_minutes.max(1);
        let (fetched, running) = {
            let mut st = self.today.lock().unwrap();
            st.opened = true;
            (st.fetched, st.running)
        };
        if !running && fetched.is_none_or(|f| now().saturating_sub(f) >= mins * 60) {
            self.today_refresh();
        }
        let (v, mut meta) = self.today_view_of(job);
        let many = v.repos.len() != 1;
        let mut out = serde_json::to_value(&v).map_err(|e| e.to_string())?;
        if let (Some(o), Some(m)) = (out.as_object_mut(), meta.as_object_mut()) {
            o.append(m);
            o.insert("many_repos".into(), json!(many));
        }
        Ok(out)
    }

    pub fn today_refresh_now(&self) -> Res<()> {
        self.today_refresh();
        Ok(())
    }

    fn today_key(number: u64, changed: bool) -> u64 {
        if changed {
            number + CHANGED_KEY
        } else {
            number
        }
    }

    /// Done, or its undo (SPEC 39.2). Done keeps the newest event of the PR, so
    /// only a newer one brings it back.
    pub fn today_done(&self, repo: &str, number: u64, changed: bool, on: bool) -> Res<()> {
        let (items, _) = self.today_items();
        let event = items
            .iter()
            .filter(|i| i.repo.eq_ignore_ascii_case(repo) && i.number == number)
            .map(|i| i.last_event)
            .max();
        let seen = if on { Seen { done_at: Some(now()), done_event: event, later_until: None } } else { Seen::default() };
        self.store.lock().unwrap().today_set(repo, Self::today_key(number, changed), &seen).map_err(|e| e.to_string())?;
        self.sink.emit("today", json!({ "refreshing": self.today.lock().unwrap().running }));
        Ok(())
    }

    /// Later until a time, or `None` to bring it back.
    pub fn today_later(&self, repo: &str, number: u64, changed: bool, until: Option<u64>) -> Res<()> {
        let seen = Seen { later_until: until, ..Seen::default() };
        self.store.lock().unwrap().today_set(repo, Self::today_key(number, changed), &seen).map_err(|e| e.to_string())?;
        self.sink.emit("today", json!({ "refreshing": self.today.lock().unwrap().running }));
        Ok(())
    }

    pub fn today_looked(&self, job: &str) -> Res<()> {
        self.store.lock().unwrap().set(&format!("today.looked:{job}"), &now().to_string()).map_err(|e| e.to_string())
    }

    /// The job for a folder: the first job that holds one of its GitHub repos.
    fn today_job_for(&self, dir: &Path) -> String {
        let cfg = self.config().today;
        let repos = crate::repo_context::repos_of(dir);
        cfg.job_names()
            .into_iter()
            .skip(1)
            .find(|j| cfg.job_repos(j).iter().any(|r| repos.iter().any(|x| x.eq_ignore_ascii_case(r))))
            .unwrap_or_else(|| EVERYTHING.to_owned())
    }

    /// Waits for a first list, at most a minute, for `sb today` and the pane tools.
    fn today_wait(&self) {
        let loaded = self.today.lock().unwrap().data.is_some();
        if loaded {
            return;
        }
        self.today_refresh();
        for _ in 0..120 {
            std::thread::sleep(Duration::from_millis(500));
            if !self.today.lock().unwrap().running {
                return;
            }
        }
    }

    /// `sb today` and the pane tools (SPEC 39.8). With a pane, the job of its repo.
    pub(crate) fn today_socket(&self, pane: Option<&str>, job: Option<&str>, item: Option<(String, u64)>) -> Value {
        self.today_wait();
        let job = match (job, pane) {
            (Some(j), _) => j.to_owned(),
            (None, Some(p)) => {
                let dir = self.panes.lock().unwrap().iter().find(|x| x.row.id == p).map(|x| x.row.tree.clone());
                match dir {
                    Some(d) => self.today_job_for(Path::new(&d)),
                    None => EVERYTHING.to_owned(),
                }
            }
            (None, None) => EVERYTHING.to_owned(),
        };
        let (v, meta) = self.today_view_of(&job);
        if let Some((repo, number)) = item {
            let found = v.sections.iter().flat_map(|s| &s.items).chain(&v.done).chain(&v.later).find(|i| i.repo.eq_ignore_ascii_case(&repo) && i.number == number);
            return match found {
                Some(i) => json!({ "ok": true, "item": today::item_json(i) }),
                None => json!({ "ok": false, "error": format!("{repo}#{number} is not on Today's list for {job}.") }),
            };
        }
        if meta["loaded"] != json!(true) {
            let why = meta["error"].as_str().unwrap_or("GitHub did not answer");
            return json!({ "ok": false, "error": format!("Today did not load: {why}") });
        }
        let mut text = today::text(&v, v.repos.len() != 1);
        if let Some(e) = meta["error"].as_str() {
            text.push_str(&format!("\nNote: {e}\n"));
        }
        json!({ "ok": true, "job": job, "text": text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real run against GitHub, read only:
    /// `cargo test -p switchboard-core live_today -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_today() {
        let cfg = crate::config::Config::load(&crate::paths::Paths::from_env().config).unwrap_or_default().today;
        let gh = crate::gh::RealGh;
        let me = cfg.user.clone().unwrap_or_else(|| gh.login().unwrap());
        let t = now();
        let t0 = std::time::Instant::now();
        let (data, err) = fetch(&gh, &me, &cfg, t);
        let data = data.expect("no data");
        let items = today::items(&data, &[], &Ctx { cfg: &cfg, me: me.to_lowercase(), now: t });
        let v = today::view(&items, &cfg, EVERYTHING, &Default::default(), None, t);
        println!("{} ms, {} open, {} closed, error {err:?}\n{}", t0.elapsed().as_millis(), data.open.len(), data.closed.len(), today::text(&v, true));
    }

    #[test]
    fn ymd_matches_known_days() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(crate::chat::epoch("2026-11-23T12:00:00Z").unwrap()), "2026-11-23");
        assert_eq!(ymd(crate::chat::epoch("2000-02-29T23:59:59Z").unwrap()), "2000-02-29");
        assert_eq!(ymd(crate::chat::epoch("2024-03-01T00:00:00Z").unwrap()), "2024-03-01");
    }

    #[test]
    fn searches_cover_the_buckets() {
        let mut cfg = today::TodayCfg::default();
        cfg.repos.insert("sigp/lighthouse".into(), today::RepoRules { areas: [("store".into(), today::AreaCfg::default())].into(), ..Default::default() });
        let t = crate::chat::epoch("2026-11-23T12:00:00Z").unwrap();
        let (open, area, closed) = searches("eserilev", &cfg, t);
        assert_eq!(open[0], "is:pr is:open archived:false author:eserilev");
        assert!(open[2].ends_with("updated:>=2026-10-24"));
        assert_eq!(area, ["repo:sigp/lighthouse is:pr is:open -author:eserilev updated:>=2026-10-24"]);
        assert_eq!(closed, "is:pr author:eserilev is:closed closed:>=2026-11-20");
    }
}
