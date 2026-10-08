//! The Switchboard window.
//!
//! PR 1 shows one live pane: the first pane of the tmux session. Every
//! other pane is paused. The board with many tiles comes in PR 3.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use switchboard_core::tmux::{Event, Tmux, CONF};
use tauri::{Emitter, Manager, State};

const SOCKET: &str = "switchboard";

struct App {
    tmux: Arc<Tmux>,
    live: Arc<Mutex<Option<String>>>,
}

#[derive(Serialize, Clone)]
struct Output {
    pane: String,
    data: String,
}

#[derive(Serialize)]
struct Expanded {
    pane: String,
    /// The scrollback and the screen, base64, ready for `term.write`.
    screen: String,
}

#[derive(Serialize)]
struct Info {
    command: String,
    path: String,
}

type Res<T> = Result<T, String>;

fn live(app: &App) -> Res<String> {
    app.live
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no live pane".to_owned())
}

/// Draws the live pane from scratch, then starts its stream.
#[tauri::command]
async fn pane_expand(app: State<'_, App>) -> Res<Expanded> {
    let pane = live(&app)?;
    let t = &app.tmux;
    t.set_streaming(&pane, false).map_err(|e| e.to_string())?;
    let lines = t.capture(&pane, "-").map_err(|e| e.to_string())?;
    let (x, y) = t.cursor(&pane).map_err(|e| e.to_string())?;
    t.set_streaming(&pane, true).map_err(|e| e.to_string())?;
    // The last lines are the screen. Put the cursor where tmux has it.
    let mut screen = lines.join("\r\n").into_bytes();
    screen.extend(format!("\x1b[{};{}H", y + 1, x + 1).into_bytes());
    Ok(Expanded {
        pane,
        screen: B64.encode(screen),
    })
}

/// Keys from xterm.js. `binary` data (mouse reports) is one byte per char.
#[tauri::command]
async fn pane_input(app: State<'_, App>, data: String, binary: bool) -> Res<()> {
    let pane = live(&app)?;
    let bytes: Vec<u8> = if binary {
        data.chars().map(|c| c as u8).collect()
    } else {
        data.into_bytes()
    };
    app.tmux
        .send_bytes(&pane, &bytes)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn pane_resize(app: State<'_, App>, cols: u16, rows: u16) -> Res<()> {
    app.tmux.set_size(cols, rows).map_err(|e| e.to_string())
}

#[tauri::command]
async fn pane_info(app: State<'_, App>) -> Res<Info> {
    let pane = live(&app)?;
    let line = app
        .tmux
        .display(&pane, "#{pane_current_command}\t#{pane_current_path}")
        .map_err(|e| e.to_string())?;
    let (command, path) = line.split_once('\t').unwrap_or((&line, ""));
    Ok(Info {
        command: command.to_owned(),
        path: path.to_owned(),
    })
}

fn conf_path() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("switchboard");
    std::fs::create_dir_all(&dir)?;
    let conf = dir.join("tmux.conf");
    std::fs::write(&conf, CONF)?;
    Ok(conf)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let live: Arc<Mutex<Option<String>>> = Arc::default();
            let stream_live = Arc::clone(&live);
            let tmux = Tmux::start(SOCKET, &conf_path()?, move |event| match event {
                Event::Output { pane, data } => {
                    if stream_live.lock().unwrap().as_deref() == Some(pane.as_str()) {
                        let _ = handle.emit(
                            "pane_output",
                            Output {
                                pane,
                                data: B64.encode(data),
                            },
                        );
                    }
                }
                Event::Exit(reason) => {
                    let _ = handle.emit("tmux_exit", reason);
                }
                _ => {}
            })?;
            let panes = tmux.panes()?;
            for p in panes.iter().skip(1) {
                tmux.set_streaming(p, false)?;
            }
            *live.lock().unwrap() = panes.into_iter().next();
            app.manage(App { tmux, live });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pane_expand,
            pane_input,
            pane_resize,
            pane_info
        ])
        .run(tauri::generate_context!())
        .expect("the window failed to start");
}
