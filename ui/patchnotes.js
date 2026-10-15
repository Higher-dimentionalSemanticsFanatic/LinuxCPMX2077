// Patch-note check: each installed mod's last update (Nexus page or newest
// release; the installed file's date until those are read) against the
// game's official patch notes (bundled list), with notes on reworked
// systems, features a patch added, and frameworks. Also a badge on the mod's
// row in the list. Hints only; nothing is turned off.
// Loaded after app.js and built from its helpers (el, $, busy, docLink, …).
// Names, summaries and notes are inserted as text, never as HTML.

let patchAges = new Map(); // installed mod id -> ModAge
let patchAutoRead = false; // release dates read once per session on opening

const PATCH_DATE_SOURCE = { nexus_page: "Nexus page", release: "newest release", file: "installed file" };

async function loadPatchNotes(readDates = false) {
  const box = $("#patch-notes");
  if (!currentGame) return box.classList.add("hidden");
  const gameId = currentGame.id;
  const r = await invoke(readDates ? "patch_notes_read_dates" : "patch_notes_report", { gameId });
  if (currentGame?.id !== gameId) return;
  patchAges = new Map(r.mods.map((m) => [m.mod_id, m]));
  box.classList.remove("hidden");
  renderPatchNotes(r);
  renderMods();
  // Opening the section without release dates: read them once.
  if (!readDates && !r.dates_read_at && !patchAutoRead && modsSub === "patchnotes" && r.mods.length) {
    patchAutoRead = true;
    await loadPatchNotes(true);
  }
}

// "updated before 2.1" for the mod's row in the list.
function patchBadge(m) {
  const a = patchAges.get(m.id);
  if (!a) return null;
  const fw = a.notes.find((n) => n.kind === "framework");
  const unlisted = a.notes.some((n) => n.kind === "unlisted");
  const label = fw ? `predates game ${fw.patch}` : a.patches_since.length ? `updated before ${a.patches_since[0]}` : unlisted ? "unlisted on Nexus" : null;
  if (!label) return null;
  const title = [patchSince(a), ...a.notes.map((n) => n.text)].join("\n");
  return el("div", {}, el("span", { class: `badge patch-age ${a.severity === "warning" ? "bad" : ""}`, title }, label));
}

function patchDate(a) {
  return `${a.date_before ? "before " : ""}${new Date(a.last_update * 1000).toISOString().slice(0, 10)}`;
}

function patchSince(a) {
  const n = a.patches_since.length;
  if (!n) return "Updated after every game patch installed";
  return `Last update before game ${a.patches_since[0]} (${a.first_patch_date}); ${n} game patch${n === 1 ? "" : "es"} since`;
}

function patchRow(a) {
  return el("tr", {},
    el("td", {}, el("b", {}, a.mod_name), a.framework ? el("div", { class: "muted small" }, "core framework") : null),
    el("td", { class: "nowrap" }, patchDate(a), el("div", { class: "muted small" }, PATCH_DATE_SOURCE[a.date_source])),
    el("td", {},
      el("div", {}, a.patches_since.length ? [`before `, el("b", {}, a.patches_since[0]),
        el("span", { class: "muted" }, ` · ${a.patches_since.length} patch${a.patches_since.length === 1 ? "" : "es"} since`)] : "current"),
      ...a.notes.map((n) => el("div", { class: "muted small" }, n.text, n.link ? " " : null,
        n.link ? docLink(n.link, [n.kind === "framework" ? "Modding wiki" : n.kind === "unlisted" ? "Mod page" : `Patch ${n.patch} notes`]) : null))),
    el("td", {}, a.severity === "warning" ? el("span", { class: "badge bad" }, "warning") : null),
    el("td", { class: "actions" }, a.link && a.patches_since.length ? docLink(a.link, [`Patch ${a.patches_since[0]} notes`]) : null));
}

function patchTable(list) {
  return el("table", { class: "req-table" },
    el("thead", {}, el("tr", {}, el("th", {}, "Mod"), el("th", {}, "Last update"), el("th", {}, "Game patches since"), el("th", {}), el("th", {}))),
    el("tbody", {}, ...list.map(patchRow)));
}

function renderPatchNotes(r) {
  const behind = r.mods.filter((a) => a.patches_since.length);
  const current = r.mods.filter((a) => !a.patches_since.length);
  const warn = r.mods.filter((a) => a.severity === "warning").length;
  const summary = [
    r.mods.length ? `${behind.length} of ${r.mods.length} mods last updated before a game patch` : "no dated mods",
    warn ? `${warn} warning${warn === 1 ? "" : "s"}` : "no warnings",
    r.game_version ? `game ${r.game_version}` : "game version unknown: all patches applied",
  ];
  const dates = r.dates_read_at
    ? `Last updates read from Nexus and GitHub ${fmtAgo(r.dates_read_at)}.`
    : "Last updates not read: dates are installed file dates (Nexus file names) until Read release dates runs.";
  $("#patch-notes").replaceChildren(...[
    el("p", {}, el("b", {}, "Patch-note check"), el("span", { class: "muted" }, ` · ${summary.join(" · ")}`)),
    el("p", { class: "muted small" },
      `Each mod's last update compared with the official patch notes up to ${r.newest_patch} (list checked ${r.data_checked}). `, dates,
      " Warnings: script, CET, plugin or TweakXL mods last updated before Update 2.0's rework, and core frameworks older than a game patch with a newer release out. Hints only: no mod is changed or disabled."),
    el("div", { class: "row" },
      el("button", { title: "Read each mod's last update, summary and page status from Nexus and GitHub", onclick: (e) => busy(e.target, async () => {
        await loadPatchNotes(true);
        toast("Last update dates read");
      }) }, "Read release dates")),
    r.errors.length ? el("p", { class: "muted small" }, "Not read: " + r.errors.join("; ")) : null,
    behind.length ? patchTable(behind) : el("p", { class: "muted" }, r.mods.length
      ? "Every dated mod was updated after the newest game patch installed."
      : "No dated mods: no installed file or release date read yet."),
    current.length ? el("details", {}, el("summary", {}, `Updated after game ${r.game_version || r.newest_patch} (${current.length})`), patchTable(current)) : null,
    r.undated.length ? el("p", { class: "muted small" },
      `Not checked, no file or release date (${r.undated.length}): ${r.undated.join(", ")}`) : null,
  ].filter(Boolean));
}
