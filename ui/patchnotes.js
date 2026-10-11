// Patch-note check: a card in Installed mods that compares each installed
// mod's dates with the game's official patch notes (bundled list), plus a
// badge on the affected mod rows. Hints only; nothing is turned off.
// Loaded after app.js and built from its helpers (el, $, busy, docLink, …).
// Names, summaries and notes are inserted as text, never as HTML.

let patchFindings = new Map(); // installed mod id -> findings, worst first

const PATCH_KIND_BADGE = {
  framework: (f) => `predates game ${f.patch}`,
  rework: (f) => `before game ${f.patch}`,
  feature: (f) => `in base game ${f.patch}?`,
  unlisted: () => "unlisted on Nexus",
};

async function loadPatchNotes(readDates = false) {
  const box = $("#patch-notes");
  if (!currentGame) return box.classList.add("hidden");
  const gameId = currentGame.id;
  const r = await invoke(readDates ? "patch_notes_read_dates" : "patch_notes_report", { gameId });
  if (currentGame?.id !== gameId) return;
  patchFindings = new Map();
  for (const f of r.findings) {
    if (!patchFindings.has(f.mod_id)) patchFindings.set(f.mod_id, []);
    patchFindings.get(f.mod_id).push(f);
  }
  box.classList.remove("hidden");
  renderPatchNotes(r);
  renderMods();
}

// The worst finding of a mod, as a badge for its row in the mod list.
function patchBadge(m) {
  const f = patchFindings.get(m.id)?.[0];
  if (!f) return null;
  const all = patchFindings.get(m.id).map((x) => `${x.title}: ${x.detail}`).join("\n\n");
  return el("div", {}, el("span", { class: `badge ${f.severity === "warning" ? "bad" : ""}`, title: all }, PATCH_KIND_BADGE[f.kind](f)));
}

function patchLink(f) {
  if (!f.link) return null;
  return docLink(f.link, [f.kind === "unlisted" ? "Mod page" : f.kind === "framework" ? "Modding wiki" : `Patch ${f.patch} notes`]);
}

function renderPatchNotes(r) {
  const warn = r.findings.filter((f) => f.severity === "warning").length;
  const hints = r.findings.length - warn;
  const summary = [
    warn ? `${warn} warning${warn === 1 ? "" : "s"}` : "no warnings",
    `${hints} hint${hints === 1 ? "" : "s"}`,
    r.game_version ? `game ${r.game_version}` : "game version unknown: all patches applied",
  ];
  const dates = r.dates_read_at
    ? `Release dates read from Nexus and GitHub ${fmtAgo(r.dates_read_at)}.`
    : "Release dates not read yet: file dates from Nexus file names only.";
  const details = el("details", { open: "" },
    el("summary", {}, el("b", {}, "Patch-note check"), el("span", { class: "muted" }, ` · ${summary.join(" · ")}`)),
    el("p", { class: "muted small" },
      `Mod dates compared with the official patch notes up to ${r.newest_patch} (list checked ${r.data_checked}). `, dates,
      " Hints only: no mod is changed or disabled."),
    el("div", { class: "row" },
      el("button", { title: "Read each mod's newest release date, summary and page status from Nexus and GitHub", onclick: (e) => busy(e.target, async () => {
        await loadPatchNotes(true);
        toast("Release dates read");
      }) }, "Read release dates")),
    r.errors.length ? el("p", { class: "muted small" }, "Not read: " + r.errors.join("; ")) : null,
    r.findings.length ? el("table", { class: "req-table" },
      el("tbody", {}, ...r.findings.map((f) => el("tr", {},
        el("td", {}, el("b", {}, f.mod_name)),
        el("td", {}, el("span", { class: `badge ${f.severity === "warning" ? "bad" : ""}` }, f.severity)),
        el("td", {}, el("div", {}, f.title), el("div", { class: "muted small" }, f.detail)),
        el("td", { class: "actions" }, patchLink(f))))))
      : el("p", { class: "muted" }, `No findings for ${r.dated} dated mod${r.dated === 1 ? "" : "s"}.`),
    r.undated.length ? el("p", { class: "muted small" },
      `Not checked, no file or release date (${r.undated.length}): ${r.undated.join(", ")}`) : null);
  $("#patch-notes").replaceChildren(details);
}
