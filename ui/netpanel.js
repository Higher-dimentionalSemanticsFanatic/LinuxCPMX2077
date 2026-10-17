// Netrunner panel: docked under Installed mods › Overview (List or Canvas).
// Its tabs (Summary, Files, Conflicts, Mod interaction, Crashes & logs,
// Launch options, WolvenKit) follow the mods selected above: a clicked List
// row, Canvas tile or graph node, or the List's ticked rows. Hidden, it is a
// one-line strip with the counts of the last check.
(() => {
  const panel = $("#tab-netrunner");
  const strip = $("#net-strip");
  const PANES = ["summary", "files", "conflicts", "graph", "crashes", "launch", "wolvenkit"];
  const STRIP_H = 34;
  let open = loadPref("netPanel.open", true) !== false;
  let pane = PANES.includes(loadPref("netPanel.pane", "summary")) ? loadPref("netPanel.pane", "summary") : "summary";
  let height = Number(loadPref("netPanel.height", 0)) || 0;
  let shown = false; // Overview is the page on screen
  let focus = null; // mod id clicked last (List row, Canvas tile, graph node, File map)
  let checkedGame = null, checking = false;

  const defaultHeight = () => Math.round(window.innerHeight * 0.46);
  const clampHeight = (h) => Math.max(160, Math.min(window.innerHeight - 180, h));

  // The mods the panel is about: the ticked List rows, else the clicked mod.
  function scopeIds() {
    const picked = [...modSel.ids].filter((id) => mods.some((m) => m.id === id));
    if (picked.length) return picked;
    return focus != null && mods.some((m) => m.id === focus) ? [focus] : [];
  }
  window.netScope = () => { const ids = scopeIds(); return ids.length ? new Set(ids) : null; };
  const nameOf = (id) => mods.find((m) => m.id === id)?.name || `Mod ${id}`;

  // ---- layout -------------------------------------------------------------
  function place() {
    const root = document.documentElement.style;
    const visible = shown;
    panel.classList.toggle("hidden", !(visible && open));
    strip.classList.toggle("hidden", !(visible && !open));
    document.body.classList.toggle("np-shown", visible);
    if (!visible) { root.removeProperty("--np-bottom"); return; }
    const h = open ? clampHeight(height || defaultHeight()) : STRIP_H;
    root.setProperty("--np-h", `${h}px`);
    root.setProperty("--np-bottom", `${h}px`);
    root.setProperty("--np-left", `${$("main").getBoundingClientRect().left}px`);
  }
  new ResizeObserver(place).observe($("main"));
  window.addEventListener("resize", place);

  function setOpen(on) {
    open = on;
    savePref("netPanel.open", on);
    place();
    if (on) { showPane(pane); ensureChecked(); }
  }

  function showPane(p) {
    pane = PANES.includes(p) ? p : "summary";
    savePref("netPanel.pane", pane);
    document.querySelectorAll("#np-tabs button").forEach((b) => {
      const on = b.dataset.pane === pane;
      b.classList.toggle("active", on);
      b.setAttribute("aria-selected", String(on));
    });
    document.querySelectorAll("#np-body .np-pane").forEach((x) => x.classList.toggle("active", x.dataset.pane === pane));
    if (pane === "launch") window.loadLaunchExplain?.();
  }

  // Open the panel on a tab; `target` (an element id) is scrolled into view.
  window.netPanelShow = (p, target) => {
    if (!$("#tab-mods").classList.contains("active") || modsSub !== "list") showTab("mods", "list");
    if (!open) setOpen(true);
    showPane(p);
    if (target) requestAnimationFrame(() => document.getElementById(target)?.scrollIntoView({ behavior: "smooth", block: "start" }));
  };
  window.netPanelIsOpen = () => shown && open;

  // Called by showTab: the panel belongs to Installed mods › Overview.
  window.netPanelSync = () => {
    shown = $("#tab-mods").classList.contains("active") && modsSub === "list";
    place();
    if (shown) ensureChecked();
  };

  // Run the checks once per game; Check again repeats them.
  function ensureChecked() {
    if (!shown || !currentGame || checking || checkedGame === currentGame.id) return;
    checkedGame = currentGame.id;
    checking = true;
    busy(null, runDiagnostics).finally(() => { checking = false; renderCounts(); });
  }
  window.netPanelChecked = () => { checkedGame = currentGame?.id ?? null; renderCounts(); applyScope("check"); };

  // ---- counts ---------------------------------------------------------------
  function renderCounts() {
    const a = lastDiag?.analysis, c = lastDiag?.crash;
    const errs = a ? a.findings.filter((f) => f.severity === "error").length : 0;
    const warns = a ? a.findings.filter((f) => f.severity === "warning").length : 0;
    const logErrs = c ? c.issues.filter((i) => i.level === "error" && i.last_session).length : 0;
    const setup = (typeof setupChecks !== "undefined" ? setupChecks.filter((x) => x.state === "problem").length : 0) + (currentGame?.install.warnings.length || 0);
    const badge = (b, n, title) => {
      b.querySelector(".np-n")?.remove();
      if (n) b.append(el("span", { class: "np-n", title }, n));
    };
    badge($('#np-tabs [data-pane="summary"]'), setup, `${setup} setup problem${setup === 1 ? "" : "s"}`);
    badge($('#np-tabs [data-pane="conflicts"]'), errs + warns, `${errs} problems, ${warns} overlaps to check`);
    badge($('#np-tabs [data-pane="crashes"]'), logErrs, `${logErrs} log errors from the last session`);
    const sel = scopeIds();
    const item = (cls, text) => el("span", { class: `s ${cls}` }, text);
    strip.replaceChildren(...[
      el("span", { class: "net-title", "data-text": "NETRUNNER" }, "NETRUNNER"),
      !lastDiag ? item("muted", checking ? "Checking…" : "Not checked")
        : [
          item(errs ? "bad" : "ok", `■ ${errs} problem${errs === 1 ? "" : "s"}`),
          item(warns ? "warn" : "ok", `■ ${warns} overlap${warns === 1 ? "" : "s"}`),
          item(logErrs ? "bad" : "ok", `■ ${logErrs} log error${logErrs === 1 ? "" : "s"}`),
          setup ? item("warn", `■ ${setup} setup`) : null,
        ],
      sel.length ? item("sel", sel.length === 1 ? `selected: ${nameOf(sel[0])}` : `${sel.length} selected`) : null,
      el("span", { class: "spacer" }),
      el("button", { tabindex: "-1" }, "Show ▴")].flat().filter(Boolean));
  }

  // ---- selection --------------------------------------------------------------
  // `from` names the view the click came from, so it is not told again.
  window.netSelectMod = (id, from) => {
    if (from === "clear") return clearScope();
    focus = id == null ? null : (from === "list" || from === "canvas") && focus === id ? null : id;
    applyScope(from);
  };
  // Ticking List rows re-renders the list; only a changed set is applied.
  let lastKey = "";
  window.netScopeChanged = () => { if (scopeIds().join(",") !== lastKey) applyScope("pick"); };

  function applyScope(from) {
    const ids = scopeIds();
    lastKey = ids.join(",");
    window.netDecorate();
    renderCounts();
    const scope = $("#np-scope");
    if (!ids.length) scope.replaceChildren("All mods");
    else scope.replaceChildren(el("b", {}, ids.length === 1 ? nameOf(ids[0]) : `${ids.length} selected mods`),
      el("button", { class: "link inline", title: "Show all mods again", onclick: () => clearScope() }, "× all mods"));
    if (lastDiag?.analysis && from !== "check") renderCompat(lastDiag.analysis);
    if (lastDiag?.crash && from !== "check") renderCrash(lastDiag.crash);
    if (from !== "graph") {
      if (ids.length === 1) window.graphSelectMod?.(ids[0]);
      else if (ids.length) window.highlightGraph?.(ids, `${ids.length} selected mods`);
      else window.graphClearSelection?.();
    }
    if (from !== "files") window.fileMapScope?.(ids);
  }

  function clearScope() {
    focus = null;
    if (modSel.ids.size) modSel.clear();
    applyScope("clear");
  }

  // Marks the scoped mods in the List and the Canvas (after each render).
  window.netDecorate = () => {
    const ids = new Set(scopeIds());
    document.querySelectorAll("#mods-body tr[data-mod-id]").forEach((tr) => tr.classList.toggle("np-sel", ids.has(Number(tr.dataset.modId))));
    document.querySelectorAll("#mods-canvas [data-mod-id]").forEach((t) => t.classList.toggle("np-sel", ids.has(Number(t.dataset.modId))));
    if (shown) ensureChecked();
  };

  // A click on a List row (not on its buttons, switches or tags) selects it.
  $("#mods-body").addEventListener("click", (e) => {
    if (e.target.closest("button, input, select, a, label, summary, details, .tag-cell")) return;
    const tr = e.target.closest("tr[data-mod-id]");
    if (tr) window.netSelectMod(Number(tr.dataset.modId), "list");
  });

  // ---- controls ---------------------------------------------------------------
  document.querySelectorAll("#np-tabs button").forEach((b) => b.addEventListener("click", () => showPane(b.dataset.pane)));
  $("#np-hide").addEventListener("click", () => setOpen(false));
  strip.addEventListener("click", () => setOpen(true));
  strip.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setOpen(true); } });

  // Drag the top edge to resize; double-click resets.
  const splitter = $("#np-splitter");
  splitter.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    splitter.setPointerCapture(e.pointerId);
    document.body.classList.add("np-resizing");
    const move = (ev) => { height = clampHeight(window.innerHeight - ev.clientY); place(); };
    const up = () => {
      splitter.removeEventListener("pointermove", move);
      document.body.classList.remove("np-resizing");
      savePref("netPanel.height", height);
    };
    splitter.addEventListener("pointermove", move);
    splitter.addEventListener("pointerup", up, { once: true });
  });
  splitter.addEventListener("dblclick", () => { height = 0; savePref("netPanel.height", 0); place(); });
  splitter.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    height = clampHeight((height || defaultHeight()) + (e.key === "ArrowUp" ? 40 : -40));
    savePref("netPanel.height", height);
    place();
  });

  showPane(pane);
  renderCounts();
  window.netPanelSync();
})();
