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

  window.SBPhone = { pair };
})();
