//! Tasks on a real hub (SPEC 27): each change sends the list to the window, a refused
//! change sends nothing, and the tasks stay in the store file.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::tasks::{NewSprint, NewTask, Place, TaskMove};

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Sink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

impl Events {
    fn tasks(&self) -> Vec<Value> {
        self.0.lock().unwrap().iter().filter(|(e, _)| e == "tasks").map(|(_, v)| v.clone()).collect()
    }
}

#[test]
fn task_changes_reach_the_window_and_stay() {
    let root = std::env::temp_dir().join(format!("sb-tasks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", format!("sb-tasks-{}", std::process::id()));
    std::env::remove_var("SB_SOCK");
    struct Kill(String, PathBuf);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _kill = Kill(std::env::var("SB_TMUX_SOCKET").unwrap(), root.clone());
    // Two tasks in the archive before the start: one deleted more than 30 days ago,
    // one less.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let paths = switchboard_core::paths::Paths::from_env();
    paths.create_all().unwrap();
    let store = switchboard_core::store::Store::open(&paths.store()).unwrap();
    let old = store.task_add(NewTask { title: "old".into(), ..NewTask::default() }, "you", 1).unwrap();
    store.task_delete(&old.id, "you", now - 31 * 86_400).unwrap();
    let new = store.task_add(NewTask { title: "new".into(), ..NewTask::default() }, "you", 2).unwrap();
    store.task_delete(&new.id, "you", now - 29 * 86_400).unwrap();
    drop(store);
    let events = Arc::new(Events::default());
    let hub = Hub::start(switchboard_core::paths::Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), events.clone()).unwrap();
    // At hub start, the old one goes with its history. Restore brings the other back.
    let v = hub.tasks().unwrap();
    assert_eq!(v.archive.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["new"], "the prune at start");
    assert!(hub.task_history(&old.id).unwrap().is_empty(), "the history goes too");
    hub.task_restore(&new.id).unwrap();
    let last = events.tasks().pop().unwrap();
    assert!(last["archive"].as_array().unwrap().is_empty() && last["tasks"].as_array().unwrap().iter().any(|t| t["title"] == "new"));
    assert!(hub.task_restore(&new.id).is_err(), "a second restore is refused");
    assert_eq!(events.tasks().len(), 1, "and sends nothing");
    hub.task_delete(&new.id).unwrap();
    events.0.lock().unwrap().clear();

    let sprint = hub
        .sprint_new(NewSprint { name: "S1".into(), goal: "".into(), start: "2026-10-12".into(), end: "2026-10-25".into() })
        .unwrap();
    let t = hub.task_add(NewTask { title: "Write tests".into(), sprint: Some(sprint.id.clone()), ..NewTask::default() }).unwrap();
    hub.task_move(&t.id, TaskMove { sprint: Some(sprint.id.clone()), status: Some("doing".into()), ..TaskMove::default() }).unwrap();
    let sent = events.tasks();
    assert_eq!(sent.len(), 3, "one event for each change");
    assert_eq!(sent[2]["tasks"][0]["status"], "doing");
    // A refused change sends nothing.
    assert!(hub.task_status(&t.id, "blocked", None).is_err());
    assert!(hub.sprint_close(&sprint.id, vec![], None).is_err());
    assert_eq!(events.tasks().len(), 3);
    hub.sprint_close(&sprint.id, vec![(t.id.clone(), Place::Backlog)], None).unwrap();
    assert_eq!(events.tasks().len(), 4);
    drop(hub);

    // The store file has the same tasks, sprints and history, for the next start.
    let store = switchboard_core::store::Store::open(&switchboard_core::paths::Paths::from_env().store()).unwrap();
    let v = store.tasks_view().unwrap();
    assert_eq!((v.sprints.len(), v.sprints[0].state.as_str()), (1, "closed"));
    assert_eq!((v.tasks[0].title.as_str(), v.tasks[0].status.as_str(), v.tasks[0].sprint.clone()), ("Write tests", "doing", None));
    let kinds: Vec<String> = store.task_history(&t.id).unwrap().into_iter().map(|h| h.kind).collect();
    assert_eq!(kinds, ["created", "status", "sprint"]);
}
