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
    let events = Arc::new(Events::default());
    let hub = Hub::start(switchboard_core::paths::Paths::from_env(), PathBuf::from(env!("CARGO_BIN_EXE_sb")), events.clone()).unwrap();
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
