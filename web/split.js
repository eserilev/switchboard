// Draggable sizes (SPEC 6.7): the rule for each size is a pure function, and one
// binder drives every handle with the pointer and the keys.
(() => {
  // A side after a drag to `want` px, with the other side at `other`, in a row of
  // `total` px. The side keeps `min`, and the middle keeps `minMid` when the row has
  // room for both. A drag of one handle never moves the other side.
  function side(total, want, other, min, minMid) {
    const max = total - other - minMid;
    return Math.round(Math.max(min, Math.min(want, max)));
  }

  // Both sides for a row of `total` px, from the saved sizes. When they leave the
  // middle less than `minMid`, they shrink in proportion to what they have above
  // `min`, never below `min`. Sizes that fit stay as they are.
  function fit(total, left, right, min, minMid) {
    let l = Math.max(min, Math.round(left));
    let r = Math.max(min, Math.round(right));
    const room = total - minMid;
    if (l + r > room) {
      const spare = Math.max(0, room - 2 * min);
      const extra = l - min + (r - min);
      if (extra > 0) {
        l = min + Math.floor(((l - min) * spare) / extra);
        r = min + Math.floor(((r - min) * spare) / extra);
      }
    }
    return [l, r];
  }

  // Binds a handle (the WAI-ARIA window splitter). `axis` is "x" or "y"; `dir` is +1
  // when the size grows with the pointer, -1 when it grows against it. `size()` reads
  // the size, `apply(v)` sets a clamped size, `limits()` gives [min, max], `reset()`
  // goes back to the default, and `done(reset, drag)` runs after each change ends
  // (save, refit): `reset` after a double-click, `drag` after a pointer drag.
  // `controls` names the element the handle sizes.
  // Keys: arrows 16 px, Shift 64 px; Home and End the smallest and largest size;
  // Enter the smallest size and back. A double-click resets.
  function bind(el, { axis, dir, size, apply: set, limits, reset, done, controls }) {
    el.tabIndex = 0;
    el.setAttribute("role", "separator");
    el.setAttribute("aria-orientation", axis === "x" ? "vertical" : "horizontal");
    if (controls) el.setAttribute("aria-controls", controls);
    const aria = () => {
      const [lo, hi] = limits();
      el.setAttribute("aria-valuenow", String(Math.round(size())));
      el.setAttribute("aria-valuemin", String(Math.round(lo)));
      el.setAttribute("aria-valuemax", String(Math.round(Math.max(lo, hi))));
    };
    const apply = (v) => { set(v); aria(); };
    let before = null;
    aria();
    // The limits change with the window.
    window.addEventListener("resize", aria);
    const pos = (e) => (axis === "x" ? e.clientX : e.clientY);
    el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      const start = pos(e);
      const from = size();
      document.body.classList.add("is-sizing", axis === "x" ? "size-x" : "size-y");
      const move = (m) => apply(from + dir * (pos(m) - start));
      const up = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        window.removeEventListener("pointercancel", up);
        document.body.classList.remove("is-sizing", "size-x", "size-y");
        aria();
        done(false, true);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      window.addEventListener("pointercancel", up);
    });
    el.addEventListener("keydown", (e) => {
      const keys = axis === "x" ? { ArrowLeft: -1, ArrowRight: 1 } : { ArrowUp: -1, ArrowDown: 1 };
      const [lo, hi] = limits();
      if (e.key in keys) apply(size() + dir * keys[e.key] * (e.shiftKey ? 64 : 16));
      else if (e.key === "Home") apply(lo);
      else if (e.key === "End") apply(hi);
      else if (e.key === "Enter") {
        if (size() > lo + 1) { before = size(); apply(lo); } else if (before !== null) apply(before);
      } else return;
      e.preventDefault();
      e.stopPropagation();
      done();
    });
    el.addEventListener("dblclick", () => {
      reset();
      aria();
      done(true);
    });
  }

  // Saved sizes are a convenience for this window only.
  const load = (key) => { try { return JSON.parse(localStorage.getItem(key) || "null"); } catch (_) { return null; } };
  const save = (key, v) => { try { localStorage.setItem(key, JSON.stringify(v)); } catch (_) {} };

  window.SBSplit = { side, fit, bind, load, save };
})();
