//! Messages between agents in the hub (SPEC 40). The rules live in `mail`.
//!
//! The mailbox hub (the server, or a computer with no `remote`) keeps every
//! message in the store. Each hub hands messages to its own agents: at the end of
//! a turn through the Stop hook, or typed in when the agent is idle and you have
//! not typed in its pane for a while. Another machine's hub links to the mailbox
//! hub with `sb msg watch <machine>` over ssh: it gets the messages for its agents
//! and sends its list of agents.

use crate::hub::Hub;
use crate::mail::{self, Agent, Msg, Window};
use crate::proto::MailOp;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

/// The link of another machine, on the mailbox hub.
struct Peer {
    /// Counts links, so the end of an old link does not remove a newer one.
    gen: u64,
    agents: Vec<Agent>,
    tx: Sender<String>,
}

#[derive(Default)]
pub struct MailState {
    /// The messages that wait for each pane of this hub, by pane id.
    queue: HashMap<String, Vec<Msg>>,
    sent: HashMap<String, Window>,
    got: HashMap<String, Window>,
    /// When you last typed in each pane.
    key: HashMap<String, Instant>,
    /// Each pane's lamp and since when, as the tick saw it.
    lamp: HashMap<String, (String, Instant)>,
    /// On the mailbox hub: the links of other machines, by machine.
    peers: HashMap<String, Peer>,
    next_gen: u64,
    /// On another machine: the link to the mailbox hub, and the ids handed here.
    link: Option<Sender<String>>,
    seen: HashSet<i64>,
    /// The last agent list sent over the link.
    sent_agents: String,
}

fn now() -> u64 {
    crate::now()
}

impl Hub {
    fn mail_on(&self) -> bool {
        self.config().messages.enabled
    }

    /// This hub's machine name in addresses.
    pub fn machine(&self) -> String {
        mail::machine_name(&self.config())
    }

    /// True on the hub that keeps the mailbox: no `remote` in the config.
    fn mailbox_here(&self) -> bool {
        self.config().mail_host().is_none()
    }

    /// The agents of this hub that can get messages: Claude panes that run.
    fn own_agents(&self) -> Vec<Agent> {
        let machine = self.machine();
        self.views()
            .into_iter()
            .filter(|v| v.kind == "claude" && !matches!(v.lamp.as_str(), "ended" | "none"))
            .map(|v| Agent {
                address: format!("{}@{machine}", v.id),
                machine: machine.clone(),
                repo: v.repo.clone(),
                branch: v.branch.clone(),
                title: v.title.clone().unwrap_or_default(),
                state: if v.permit.is_some() { "needs".into() } else { v.lamp.as_str().into() },
            })
            .collect()
    }

    /// Every agent that can get messages: this hub's, and those of the linked machines.
    pub fn mail_agents(&self) -> Vec<Agent> {
        let mut list = self.own_agents();
        let st = self.mail.lock().unwrap();
        let mut peers: Vec<_> = st.peers.iter().collect();
        peers.sort_by(|a, b| a.0.cmp(b.0));
        for (_, p) in peers {
            list.extend(p.agents.iter().cloned());
        }
        list
    }

    /// The label of a pane of this hub: its repo, branch and title.
    fn label_of(&self, pane: &str) -> Option<String> {
        let v = self.views().into_iter().find(|v| v.id == pane)?;
        Some([v.repo, v.branch, v.title.unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "))
    }

    /// Saves a message on the mailbox hub and starts its way to the agent.
    pub fn mail_send(&self, from: &str, label: &str, to: &str, text: &str) -> Res<Msg> {
        if !self.mail_on() {
            return Err("Messages between agents are off ([messages] enabled = false).".into());
        }
        if !self.mailbox_here() {
            return Err("The mailbox is on the server: send with sb msg there.".into());
        }
        let (pane, machine) = mail::parse(to).ok_or(format!("{to} is not an agent address. Use one from agents_list, like p7@box."))?;
        mail::parse(from).ok_or(format!("{from} is not an agent address"))?;
        if from == to {
            return Err("An agent cannot send a message to itself.".into());
        }
        let text = mail::clean(text).ok_or("The message is empty.")?;
        let here = machine == self.machine();
        let known = if here {
            self.own_agents().iter().any(|a| a.address == to)
        } else {
            let st = self.mail.lock().unwrap();
            match st.peers.get(machine) {
                Some(p) => p.agents.iter().any(|a| a.address == to),
                // A machine with no link: the message waits for it.
                None => true,
            }
        };
        if !known {
            return Err(format!("No agent {to} runs now. agents_list shows the agents."));
        }
        {
            let mut st = self.mail.lock().unwrap();
            if !st.sent.entry(from.to_owned()).or_default().take(now(), mail::SEND_MAX, mail::SEND_WINDOW) {
                return Err(format!("{from} sent {} messages in the last {} minutes. Wait, or ask the user.", mail::SEND_MAX, mail::SEND_WINDOW / 60));
            }
        }
        let at = now();
        let id = self.store.lock().unwrap().mail_add(at, from, label, to, &text).map_err(|e| e.to_string())?;
        let msg = Msg { id, at, from: from.into(), label: label.into(), to: to.into(), text, state: "new".into() };
        tracing::info!(target: "sb::mail", id, %from, %to, "message");
        self.sink.emit("mail", json!(msg));
        if here {
            self.mail_arrive(pane, msg.clone());
        } else if let Some(p) = self.mail.lock().unwrap().peers.get(machine) {
            let _ = p.tx.send(json!({ "msg": msg }).to_string());
        }
        Ok(msg)
    }

    /// A message for a pane of this hub: it waits in the queue until the agent
    /// can take it.
    fn mail_arrive(&self, pane: &str, msg: Msg) {
        {
            let mut st = self.mail.lock().unwrap();
            if !st.seen.insert(msg.id) && !self.mailbox_here() {
                return;
            }
            let q = st.queue.entry(pane.to_owned()).or_default();
            if q.iter().any(|m| m.id == msg.id) {
                return;
            }
            q.push(msg);
        }
        self.mail_pump(pane);
    }

    /// Takes the queue of a pane when the agent may get it, and counts it. None
    /// when it is empty or the agent had too many.
    fn mail_pop(&self, pane: &str) -> Option<Vec<Msg>> {
        let mut st = self.mail.lock().unwrap();
        if st.queue.get(pane).is_none_or(|q| q.is_empty()) {
            return None;
        }
        let n = st.queue[pane].len();
        let got = st.got.entry(pane.to_owned()).or_default();
        let t = now();
        for _ in 0..n {
            if !got.take(t, mail::RECV_MAX, mail::RECV_WINDOW) {
                // Two agents in a loop stop here: the rest waits for the user.
                tracing::warn!(target: "sb::mail", pane, "too many messages: they wait");
                return None;
            }
        }
        st.queue.remove(pane)
    }

    /// The messages are with the agent: the store, or the mailbox hub, knows.
    fn mail_handed(&self, pane: &str, msgs: &[Msg]) {
        let at = now();
        for m in msgs {
            if self.mailbox_here() {
                let _ = self.store.lock().unwrap().mail_set(m.id, "handed", at);
            } else if let Some(tx) = &self.mail.lock().unwrap().link {
                let _ = tx.send(json!({ "handed": m.id }).to_string());
            }
            let mut m = m.clone();
            m.state = "handed".into();
            self.sink.emit("mail", json!(m));
        }
        tracing::info!(target: "sb::mail", pane, n = msgs.len(), "handed");
    }

    /// Types the waiting messages into an idle agent, with Enter, when you have not
    /// typed in its pane for a while (SPEC 40.4).
    pub(crate) fn mail_pump(&self, pane: &str) {
        let (lamp, settled, quiet) = {
            let lamp = self.views().into_iter().find(|v| v.id == pane).map(|v| if v.permit.is_some() { "needs".to_owned() } else { v.lamp.as_str().to_owned() });
            let Some(lamp) = lamp else { return };
            let st = self.mail.lock().unwrap();
            let settled = st.lamp.get(pane).filter(|(l, _)| *l == lamp).map_or(0, |(_, t)| t.elapsed().as_secs());
            let quiet = st.key.get(pane).map_or(u64::MAX, |t| t.elapsed().as_secs());
            (lamp, settled, quiet)
        };
        if !mail::may_type(&lamp, settled, quiet) {
            return;
        }
        let Some(msgs) = self.mail_pop(pane) else { return };
        let text = mail::wrap(&msgs);
        let typed = self
            .input(pane, crate::hub::bracketed_paste(&text).as_bytes())
            .and_then(|_| {
                // Enter on its own, after the paste, so the prompt takes the paste first.
                std::thread::sleep(Duration::from_millis(150));
                self.input(pane, b"\r")
            });
        match typed {
            Ok(()) => self.mail_handed(pane, &msgs),
            Err(err) => {
                tracing::warn!(target: "sb::mail", pane, %err, "type message");
                let mut st = self.mail.lock().unwrap();
                let q = st.queue.entry(pane.to_owned()).or_default();
                let rest = std::mem::take(q);
                *q = msgs.into_iter().chain(rest).collect();
            }
        }
    }

    /// The Stop hook of a pane (SPEC 40.4): the waiting messages, as the reason
    /// that makes the agent go on. None when nothing waits.
    pub fn mail_take(&self, pane: &str) -> Option<String> {
        if !self.mail_on() {
            return None;
        }
        let msgs = self.mail_pop(pane)?;
        self.mail_handed(pane, &msgs);
        // The hook then reports `working`, not `turn`: the agent goes on.
        Some(mail::wrap(&msgs))
    }

    /// A key from you in a pane: a message waits until you stop typing.
    pub fn mail_key(&self, pane: &str) {
        self.mail.lock().unwrap().key.insert(pane.to_owned(), Instant::now());
    }

    /// Each second: the lamps, the queues, and the agent list for the link.
    pub(crate) fn mail_tick(&self) {
        let views = self.views();
        let waiting: Vec<String> = {
            let mut st = self.mail.lock().unwrap();
            for v in &views {
                let lamp = if v.permit.is_some() { "needs".to_owned() } else { v.lamp.as_str().to_owned() };
                let e = st.lamp.entry(v.id.clone()).or_insert((lamp.clone(), Instant::now()));
                if e.0 != lamp {
                    *e = (lamp, Instant::now());
                }
            }
            st.queue.iter().filter(|(_, q)| !q.is_empty()).map(|(p, _)| p.clone()).collect()
        };
        for pane in waiting {
            if views.iter().any(|v| v.id == pane) {
                self.mail_pump(&pane);
            }
        }
        // Another machine sends its agents when they change.
        if !self.mailbox_here() {
            let list = json!({ "agents": self.own_agents() }).to_string();
            let mut st = self.mail.lock().unwrap();
            if let Some(tx) = st.link.clone() {
                if st.sent_agents != list && tx.send(list.clone()).is_ok() {
                    st.sent_agents = list;
                }
            }
        }
    }

    /// The last messages from and to a pane of this hub, newest first, for the
    /// inspector. Only the mailbox hub has them all.
    pub fn mail_list(&self, pane: &str) -> Res<Value> {
        let addr = format!("{pane}@{}", self.machine());
        let list = self.store.lock().unwrap().mail_of(&addr, 20).map_err(|e| e.to_string())?;
        Ok(json!({ "address": addr, "messages": list, "here": self.mailbox_here(), "on": self.mail_on() }))
    }

    /// One mail request on the socket.
    pub(crate) fn mail_socket(&self, op: MailOp) -> Value {
        let res = match op {
            MailOp::Send { pane, from, label, to, text } => {
                let (from, label) = match (pane, from) {
                    // A pane of this hub: its address and label come from here.
                    (Some(p), None) => match self.label_of(&p) {
                        Some(l) => (format!("{p}@{}", self.machine()), l),
                        None => return json!({ "ok": false, "error": format!("{p} is not an agent on the board") }),
                    },
                    (None, Some(f)) => (f, label.unwrap_or_default()),
                    _ => return json!({ "ok": false, "error": "a message needs its sender" }),
                };
                self.mail_send(&from, &mail::first_label(&label), &to, &text).map(|m| json!({ "ok": true, "id": m.id, "to": m.to }))
            }
            MailOp::Agents { pane } => {
                let you = pane.map(|p| format!("{p}@{}", self.machine()));
                let list: Vec<Value> = self.mail_agents().into_iter().map(|a| {
                    let mut v = json!(a);
                    v["you"] = json!(you.as_deref() == Some(a.address.as_str()));
                    v
                }).collect();
                Ok(json!({ "ok": true, "agents": list, "on": self.mail_on() }))
            }
            MailOp::Take { pane } => Ok(json!({ "ok": true, "text": self.mail_take(&pane) })),
            MailOp::Watch { .. } => Err("a watch keeps its own connection".into()),
        };
        res.unwrap_or_else(|e| json!({ "ok": false, "error": e }))
    }

    /// The link of another machine on the mailbox hub (SPEC 40.5). It gets the
    /// new messages for that machine, now and as they come, and sends its agents
    /// and what it handed. It holds the connection until the link ends.
    pub(crate) fn mail_watch(&self, machine: &str, conn: UnixStream) {
        if mail::parse(&format!("x@{machine}")).is_none() || machine == self.machine() {
            let mut w = &conn;
            let _ = writeln!(w, "{}", json!({ "error": format!("{machine} is not another machine") }));
            return;
        }
        let (tx, rx) = mpsc::channel::<String>();
        let gen = {
            let mut st = self.mail.lock().unwrap();
            st.next_gen += 1;
            let gen = st.next_gen;
            // A newer link of the same machine wins: the old one ends.
            st.peers.insert(machine.to_owned(), Peer { gen, agents: vec![], tx: tx.clone() });
            gen
        };
        tracing::info!(target: "sb::mail", machine, "link up");
        let Ok(mut w) = conn.try_clone() else { return };
        let writer = std::thread::spawn(move || {
            for line in rx {
                if writeln!(w, "{line}").and_then(|_| w.flush()).is_err() {
                    break;
                }
            }
            let _ = w.shutdown(std::net::Shutdown::Both);
        });
        let waiting = self.store.lock().unwrap().mail_new_for_machine(machine).unwrap_or_default();
        for m in waiting {
            let _ = tx.send(json!({ "msg": m }).to_string());
        }
        drop(tx);
        for line in BufReader::new(&conn).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(list) = v.get("agents").and_then(|a| serde_json::from_value::<Vec<Agent>>(a.clone()).ok()) {
                // Only the machine's own agents: a link cannot speak for another one.
                let list: Vec<Agent> = list.into_iter().filter(|a| a.machine == machine && a.address.ends_with(&format!("@{machine}"))).take(200).collect();
                if let Some(p) = self.mail.lock().unwrap().peers.get_mut(machine).filter(|p| p.gen == gen) {
                    p.agents = list;
                }
                self.sink.emit("mail_agents", Value::Null);
            }
            if let Some(id) = v.get("handed").and_then(Value::as_i64) {
                // Only a message to that machine.
                let ok = self.store.lock().unwrap().mail_new_for_machine(machine).unwrap_or_default().into_iter().find(|m| m.id == id);
                if let Some(mut m) = ok {
                    let _ = self.store.lock().unwrap().mail_set(id, "handed", now());
                    m.state = "handed".into();
                    self.sink.emit("mail", json!(m));
                }
            }
        }
        {
            let mut st = self.mail.lock().unwrap();
            if st.peers.get(machine).is_some_and(|p| p.gen == gen) {
                st.peers.remove(machine);
            }
        }
        let _ = conn.shutdown(std::net::Shutdown::Both);
        let _ = writer.join();
        self.sink.emit("mail_agents", Value::Null);
        tracing::info!(target: "sb::mail", machine, "link down");
    }

    /// On a computer with `remote`: the link to the mailbox hub, for as long as
    /// the hub runs (SPEC 40.5). It starts again after a pause when it ends.
    pub(crate) fn mail_link_start(&self) {
        let me = self.me.clone();
        let Some(argv) = self.config().mail_argv(&["watch", &self.machine()]) else { return };
        if !self.mail_on() {
            return;
        }
        std::thread::Builder::new()
            .name("sb-mail-link".into())
            .spawn(move || {
                let mut pause = Duration::from_secs(2);
                loop {
                    let Some(hub) = me.upgrade() else { break };
                    let t0 = Instant::now();
                    if let Err(err) = hub.mail_link_once(&argv) {
                        tracing::info!(target: "sb::mail", %err, "mail link ended");
                    }
                    drop(hub);
                    // A link that ran a while starts again soon; one that fails at once waits longer each time.
                    pause = if t0.elapsed() > Duration::from_secs(60) { Duration::from_secs(2) } else { (pause * 2).min(Duration::from_secs(60)) };
                    std::thread::sleep(pause);
                }
            })
            .ok();
    }

    fn mail_link_once(&self, argv: &[String]) -> Res<()> {
        let mut child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("{}: {e}", argv[0]))?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, rx) = mpsc::channel::<String>();
        {
            let mut st = self.mail.lock().unwrap();
            st.link = Some(tx);
            st.sent_agents.clear();
        }
        let writer = std::thread::spawn(move || {
            for line in rx {
                if writeln!(stdin, "{line}").and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        tracing::info!(target: "sb::mail", "mail link up");
        self.mail_tick();
        let machine = self.machine();
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(err) = v.get("error").and_then(Value::as_str) {
                tracing::warn!(target: "sb::mail", err, "mailbox refused the link");
            }
            let Some(msg) = v.get("msg").and_then(|m| serde_json::from_value::<Msg>(m.clone()).ok()) else { continue };
            let to = msg.to.clone();
            match mail::parse(&to) {
                Some((pane, m)) if m == machine && self.own_agents().iter().any(|a| a.address == to) => {
                    self.sink.emit("mail", json!(msg));
                    self.mail_arrive(pane, msg)
                }
                // The agent is gone here: it waits on the server, for a pane of that id.
                _ => {}
            }
        }
        self.mail.lock().unwrap().link = None;
        let _ = child.kill();
        let _ = child.wait();
        let _ = writer.join();
        Ok(())
    }
}
