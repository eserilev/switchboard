//! Tasks in the hub (SPEC 27). The rules live in `tasks`. Each change sends the
//! whole list to the window as a `tasks` event.

use crate::hub::Hub;
use crate::now;
use crate::tasks::{NewSprint, NewTask, Place, Sprint, Task, TaskEvent, TaskFields, TaskMove, TasksView};
use serde_json::json;

type Res<T> = Result<T, String>;

/// The window is the only caller in phase 1 (SPEC 27.13).
const BY: &str = "you";

impl Hub {
    pub fn tasks(&self) -> Res<TasksView> {
        self.store.lock().unwrap().tasks_view()
    }

    /// Sends the list after a change. Takes the store lock, so call it with no lock held.
    fn emit_tasks(&self) {
        match self.tasks() {
            Ok(v) => self.sink.emit("tasks", json!(v)),
            Err(err) => tracing::warn!(target: "sb::tasks", %err, "the task list did not load"),
        }
    }

    /// Runs one change on the store, then sends the list.
    fn task_change<X>(&self, f: impl FnOnce(&crate::store::Store) -> Res<X>) -> Res<X> {
        let out = f(&self.store.lock().unwrap());
        if out.is_ok() {
            self.emit_tasks();
        }
        out
    }

    pub fn task_add(&self, task: NewTask) -> Res<Task> {
        self.task_change(|s| s.task_add(task, BY, now()))
    }

    pub fn task_edit(&self, id: &str, fields: TaskFields) -> Res<Task> {
        self.task_change(|s| s.task_edit(id, fields, BY, now()))
    }

    pub fn task_status(&self, id: &str, status: &str, blocked_by: Option<&str>) -> Res<Task> {
        self.task_change(|s| s.task_status(id, status, blocked_by, BY, now()))
    }

    pub fn task_move(&self, id: &str, m: TaskMove) -> Res<Task> {
        self.task_change(|s| s.task_move(id, &m, BY, now()))
    }

    pub fn task_delete(&self, id: &str) -> Res<()> {
        self.task_change(|s| s.task_delete(id))
    }

    pub fn task_copy(&self, id: &str) -> Res<Task> {
        self.task_change(|s| s.task_copy(id, BY, now()))
    }

    pub fn task_history(&self, id: &str) -> Res<Vec<TaskEvent>> {
        self.store.lock().unwrap().task_history(id)
    }

    pub fn sprint_new(&self, sprint: NewSprint) -> Res<Sprint> {
        self.task_change(|s| s.sprint_new(&sprint))
    }

    pub fn sprint_edit(&self, id: &str, sprint: NewSprint) -> Res<Sprint> {
        self.task_change(|s| s.sprint_edit(id, &sprint))
    }

    pub fn sprint_close(&self, id: &str, places: Vec<(String, Place)>, next: Option<NewSprint>) -> Res<Option<Sprint>> {
        self.task_change(|s| s.sprint_close(id, &places, next.as_ref(), BY, now()))
    }
}
