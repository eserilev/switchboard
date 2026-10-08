// PR 1: one live pane. The core streams it; we draw it and send keys back.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const css = getComputedStyle(document.documentElement);
const token = (name) => css.getPropertyValue(name).trim();

const term = new Terminal({
  fontFamily: token("--mono"),
  fontSize: 13,
  cursorBlink: true,
  scrollback: 50000,
  theme: { background: token("--bg"), foreground: token("--fg"), cursor: token("--accent"), selectionBackground: "#d9b56c44" },
});
const fit = new FitAddon.FitAddon();
term.loadAddon(fit);
term.open(document.getElementById("term"));
fit.fit();

const bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));

// Output can arrive before the first draw. Hold it until the screen is drawn.
let drawn = false;
const held = [];
listen("pane_output", (e) => {
  const b = bytes(e.payload.data);
  drawn ? term.write(b) : held.push(b);
});
listen("tmux_exit", () => { document.getElementById("ended").hidden = false; });

term.onData((data) => invoke("pane_input", { data, binary: false }));
term.onBinary((data) => invoke("pane_input", { data, binary: true }));

let resizeTimer;
new ResizeObserver(() => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    fit.fit();
    invoke("pane_resize", { cols: term.cols, rows: term.rows });
  }, 60);
}).observe(document.getElementById("term"));

async function info() {
  try {
    const i = await invoke("pane_info");
    document.getElementById("name").textContent = i.path.split("/").filter(Boolean).pop() || "/";
    document.getElementById("cmd").textContent = i.command;
  } catch (_) { /* the pane is gone; tmux_exit shows it */ }
}

async function start() {
  await invoke("pane_resize", { cols: term.cols, rows: term.rows });
  const r = await invoke("pane_expand");
  term.write(bytes(r.screen));
  held.splice(0).forEach((b) => term.write(b));
  drawn = true;
  term.focus();
  info();
  setInterval(info, 2000);
}

start().catch((e) => {
  const el = document.getElementById("ended");
  el.textContent = String(e);
  el.hidden = false;
});
