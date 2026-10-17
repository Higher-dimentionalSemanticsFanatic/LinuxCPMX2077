// Installed mods › Overview: the List view (table, in app.js) or the Canvas
// view, which lays the same filtered mods out as tiles, one column per tag,
// Nexus category, source or state. A mod with several tags shows in each of
// its tag columns, with dots in its other tags' colors.

const CANVAS_PALETTE = ["#4fe3d0", "#f6cf3c", "#b48cff", "#3ddc84", "#ff9f43", "#4da3ff", "#ff6bcb", "#e8e8f0", "#9be15d", "#ff4d5e"];
const CANVAS_NONE = "#8fb0ab";
const CANVAS_GROUPS = ["tag", "taggroup", "category", "source", "state"];
let modsView = loadPref("modsView", "list") === "canvas" ? "canvas" : "list";
let canvasGroup = CANVAS_GROUPS.includes(loadPref("canvasGroup", "tag")) ? loadPref("canvasGroup", "tag") : "tag";
let canvasDisabled = loadPref("canvasDisabled", true) !== false;

function setModsView(view) {
  modsView = view === "canvas" ? "canvas" : "list";
  savePref("modsView", modsView);
  $("#tab-mods").dataset.view = modsView;
  document.querySelectorAll("#mods-view button").forEach((b) => b.classList.toggle("active", b.dataset.view === modsView));
}

function canvasSource(m) {
  if (m.source === "nexus") return "Nexus";
  if (m.source === "reshade") return "reshade.me";
  if (m.source === "optiscaler") return "GitHub";
  if (m.source === "radioext") return "Radio station";
  return sourceInfos.find((s) => s.id === m.source)?.label || "Manual";
}

// The tag's own color, else a palette color by its place in the tag list.
const canvasTagColor = (name) => {
  const i = modTags.tags.findIndex((t) => t.name === name);
  const c = modTags.tags[i]?.color;
  return c && /^#[0-9a-f]{6}$/i.test(c) ? c : CANVAS_PALETTE[Math.max(i, 0) % CANVAS_PALETTE.length];
};

// [{ key, label, color, filter, mods }] in display order; `filter` is the
// Tags and categories value that shows the same mods in the list.
function canvasColumns(list) {
  const cols = new Map();
  const add = (key, label, color, filter, m) => {
    if (!cols.has(key)) cols.set(key, { key, label, color, filter, mods: [] });
    cols.get(key).mods.push(m);
  };
  if (canvasGroup === "tag") {
    modTags.tags.forEach((t) => cols.set(`tag:${t.name}`, { key: `tag:${t.name}`, label: t.name, color: canvasTagColor(t.name), filter: `tag:${t.name}`, mods: [] }));
    for (const m of list) {
      const names = modTags.mods[m.id] || [];
      if (!names.length) add("~none", "No tags", CANVAS_NONE, `tag:${NO_CATEGORY}`, m);
      for (const n of names) add(`tag:${n}`, n, canvasTagColor(n), `tag:${n}`, m);
    }
  } else if (canvasGroup === "taggroup") {
    // One column per predefined tag set group, "Your tags" for the rest.
    const byName = new Map(modTags.tags.map((t) => [t.name, t]));
    const gname = new Map((modTags.groups || []).map((g) => [g.id, g.name]));
    for (const g of modTags.groups || []) {
      const t = modTags.tags.find((x) => x.group === g.id);
      if (t) cols.set(`grp:${g.id}`, { key: `grp:${g.id}`, label: g.name, color: canvasTagColor(t.name), filter: null, mods: [] });
    }
    for (const m of list) {
      const names = modTags.mods[m.id] || [];
      if (!names.length) add("~none", "No tags", CANVAS_NONE, `tag:${NO_CATEGORY}`, m);
      const keys = new Set(names.map((n) => gname.has(byName.get(n)?.group) ? `grp:${byName.get(n).group}` : "~own"));
      for (const k of keys) add(k, k === "~own" ? "Your tags" : gname.get(k.slice(4)), k === "~own" ? CANVAS_PALETTE[0] : null, null, m);
    }
  } else {
    const keyOf = {
      category: (m) => m.category ? [m.category, m.category, m.category] : ["~none", "No Nexus category", NO_CATEGORY],
      source: (m) => { const s = canvasSource(m); return [s, s, null]; },
      state: (m) => m.status === "installed" ? ["on", "Enabled", null] : ["off", "Disabled", null],
    }[canvasGroup];
    const rows = list.map((m) => [keyOf(m), m]);
    if (canvasGroup === "state") rows.sort((a, b) => a[0][0] === b[0][0] ? 0 : a[0][0] === "on" ? -1 : 1);
    else rows.sort((a, b) => (a[0][0] === "~none") - (b[0][0] === "~none") || a[0][1].localeCompare(b[0][1]));
    const keys = [...new Set(rows.map(([k]) => k[0]))];
    for (const [[key, label, filter], m] of rows) {
      const color = key === "~none" || key === "off" ? CANVAS_NONE : CANVAS_PALETTE[keys.indexOf(key) % CANVAS_PALETTE.length];
      add(key, label, color, filter, m);
    }
  }
  return [...cols.values()].filter((c) => c.mods.length);
}

// Opens the mod in the List view: name filter set to it, row scrolled into view.
function canvasOpenInList(m) {
  $("#mods-filter").value = m.name;
  $("#mods-category").value = "";
  setModsView("list");
  renderMods();
  document.querySelector("#mods-body tr:not(.deficit)")?.scrollIntoView({ block: "center" });
}

function canvasTile(m, col) {
  const on = m.status === "installed";
  const others = canvasGroup === "tag" ? (modTags.mods[m.id] || []).filter((n) => `tag:${n}` !== col.key) : [];
  const up = updatesByMod.get(m.id);
  const what = [m.name, on ? "enabled" : "disabled", `${m.file_count} file${m.file_count === 1 ? "" : "s"}`,
    (modTags.mods[m.id] || []).length ? `tags: ${modTags.mods[m.id].join(", ")}` : "no tags",
    m.category ? `Nexus category: ${m.category}` : null, up ? `${up.latest} available` : null].filter(Boolean).join(" · ");
  const tile = el("button", {
    class: `canvas-tile${on ? "" : " off"}`, style: `--c:${col.color}`, "data-mod-id": m.id,
    title: `${what}\nClick: select (Netrunner panel shows this mod). Double-click: show in the List view`,
    onclick: () => window.netSelectMod(m.id, "canvas"),
  },
  el("span", { class: "canvas-name" }, m.name),
  ...others.map((n) => el("span", { class: "canvas-dot", style: `background:${canvasTagColor(n)}`, "aria-label": `also ${n}` })),
  up ? el("span", { class: "canvas-up", "aria-label": "update available" }, "▲") : null,
  el("span", { class: "canvas-files" }, m.file_count));
  tile.addEventListener("dblclick", () => canvasOpenInList(m));
  return tile;
}

function renderModsCanvas(shown) {
  $("#tab-mods").dataset.view = modsView;
  if (modsView !== "canvas") return;
  const list = canvasDisabled ? shown : shown.filter((m) => m.status === "installed");
  const cols = canvasColumns(list);
  const several = list.filter((m) => (modTags.mods[m.id] || []).length > 1).length;
  $("#canvas-summary").textContent = `${list.length} mod${list.length === 1 ? "" : "s"} · ${cols.length} column${cols.length === 1 ? "" : "s"}`
    + (canvasGroup === "tag" && several ? ` · ${several} with several tags` : "");
  $("#mods-canvas").replaceChildren(...cols.map((c) => el("section", { class: "canvas-col", style: `--c:${c.color}` },
    el("h4", {},
      canvasGroup === "category" && c.label !== "No Nexus category"
        ? el("img", { class: "nx-src", src: "img/nexusmods.png", alt: "Nexus Mods category:", title: "Nexus Mods category, from the mods' Nexus pages" }) : null,
      c.filter ? el("button", { class: "link", title: "Show these mods in the List view", onclick: () => {
        $("#mods-filter").value = "";
        $("#mods-category").value = c.filter;
        setModsView("list");
        renderMods();
      } }, c.label) : el("span", {}, c.label),
      el("span", { class: "canvas-count" }, c.mods.length)),
    ...c.mods.map((m) => canvasTile(m, c)))));
}

setModsView(modsView);
$("#canvas-group").value = canvasGroup;
$("#canvas-disabled").checked = canvasDisabled;
document.querySelectorAll("#mods-view button").forEach((b) => b.addEventListener("click", () => { setModsView(b.dataset.view); renderMods(); }));
$("#canvas-group").addEventListener("change", (e) => { canvasGroup = e.target.value; savePref("canvasGroup", canvasGroup); renderMods(); });
$("#canvas-disabled").addEventListener("change", (e) => { canvasDisabled = e.target.checked; savePref("canvasDisabled", canvasDisabled); renderMods(); });
