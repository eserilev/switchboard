// The phone app's pairing screen (SPEC 41.4, 42). On a computer it does nothing.
// app.js calls SBPhone.pair() when the phone is not paired yet.
(function () {
  const { invoke } = window.__TAURI__.core;

  function el(tag, attrs, ...kids) {
    const e = document.createElement(tag);
    Object.assign(e, attrs || {});
    for (const k of kids) e.append(k);
    return e;
  }

  async function scan() {
    // The camera plugin, on a phone only. It gives the text of the QR code.
    await invoke("plugin:barcode-scanner|request_permissions").catch(() => {});
    const r = await invoke("plugin:barcode-scanner|scan", { formats: ["QR_CODE"], windowed: false });
    return r && r.content;
  }

  function pair(given) {
    document.body.classList.add("phone");
    const err = el("p", { className: "pherr", role: "alert" });
    const text = el("textarea", { className: "phtext", rows: 3, placeholder: "SWITCHBOARD1 …", ariaLabel: "The pairing code's text" });
    const scanBtn = el("button", { className: "btn primary phbig", type: "button", textContent: "Scan the code" });
    const pasteBtn = el("button", { className: "btn", type: "button", textContent: "Use this text" });
    const step1 = el(
      "div",
      { className: "phstep" },
      el("h1", { textContent: "Pair with your computer" }),
      el("p", { textContent: "On the computer that runs your agents, run sb pair. Scan the code it shows." }),
      scanBtn,
      el("details", {}, el("summary", { textContent: "No camera? Paste the code's text" }), text, pasteBtn),
    );
    // The error line stays below whichever step shows.
    const box = el("div", { className: "phpair", id: "phpair" }, step1, err);
    document.body.append(box);

    let step2 = null;
    async function start(qr) {
      err.textContent = "";
      if (!qr || !qr.trim()) return;
      scanBtn.disabled = pasteBtn.disabled = true;
      try {
        const { code } = await invoke("phone_pair_start", { qr: qr.trim() });
        step2 = el(
            "div",
            { className: "phstep" },
            el("h1", { textContent: "Check the code" }),
            el("p", { textContent: "The computer shows a code too. If they match, type yes there." }),
            el("p", { className: "phcode", textContent: code.replace(/(\d{3})(\d{3})/, "$1 $2") }),
            el("p", { className: "phwait", textContent: "Waiting for the computer…" }),
          );
        step1.replaceWith(step2);
        await invoke("phone_pair_wait");
        box.remove();
      } catch (e) {
        if (step2) step2.replaceWith(step1);
        step2 = null;
        err.textContent = String(e);
        scanBtn.disabled = pasteBtn.disabled = false;
      }
    }

    if (given) {
      text.value = given;
      start(given);
    }
    scanBtn.onclick = async () => {
      try {
        await start(await scan());
      } catch (e) {
        err.textContent = "The camera did not work: " + e + ". Paste the code's text instead.";
      }
    };
    pasteBtn.onclick = () => start(text.value);
  }

  // The hand-over to the phone screens (SPEC 42, web/m/). `ready(fn)` calls fn each
  // time the link to the hub comes up on a phone: after pairing, at start when the
  // phone is paired, and after each reconnect. The same moment fires the DOM event
  // `sb:phone-ready` on document. `call(cmd, args)` runs a hub command: the window
  // protocol of the desktop (SPEC 29.2), limited to the phone's classes (41.7).
  const readyFns = [];
  function ready(fn) {
    readyFns.push(fn);
  }
  function fire(link) {
    for (const fn of readyFns) {
      try {
        fn(link);
      } catch (e) {
        console.error(e);
      }
    }
    document.dispatchEvent(new CustomEvent("sb:phone-ready", { detail: link }));
  }
  const call = (cmd, args) => invoke(cmd, args || {});
  let isPhone = false;
  window.__TAURI__.event.listen("link", (e) => {
    if ((isPhone || document.body.classList.contains("phone")) && e.payload && e.payload.up) fire(e.payload);
  });
  // The phone screens (SPEC 42, web/m): mounted at the first link, then told when
  // the link goes up or down. The desktop page stays underneath, hidden.
  let ui = null;
  function seen() {
    return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  ready((link) => {
    if (ui) return ui.setHub({ linked: true, seen: "now" });
    if (!window.SBM) return;
    const root = el("div", { id: "mroot" });
    document.body.append(root);
    document.body.classList.add("m-on");
    ui = window.SBM.mount(root, {
      call,
      listen: (ev, fn) => window.__TAURI__.event.listen(ev, fn),
      hub: { name: link.host || "hub", linked: true },
      unpair: async () => {
        await invoke("phone_unpair");
        location.reload();
      },
    });
  });
  window.__TAURI__.event.listen("link", (e) => {
    if (ui && e.payload && !e.payload.up) ui.setHub({ linked: false, seen: seen() });
  });

  // A phone that is paired and already linked when the page loads.
  invoke("link_state")
    .then((st) => {
      isPhone = !!(st && st.phone);
      if (isPhone && st.paired && st.up) fire(st);
    })
    .catch(() => {});

  window.SBPhone = { pair, ready, call };
})();
