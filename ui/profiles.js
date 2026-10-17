// Mod profiles: named lists of enabled mods, filed under a game version.
// The ribbon in Installed mods shows each game version; under the current
// one the stock game tag (every mod disabled), and under each version a
// dropdown of its profiles. Picking a chip shows its actions below the ribbon.
// Applying a profile enables its mods and disables every other installed
// mod. Loaded after app.js and built from its helpers (el, $, busy, …).
// Profile and mod names are inserted as text, never as HTML.

let profiles = [];
let profilePick = null; // id of the selected profile, "stock", "new" or null
let profileRenaming = false;
const profilesOpen = new Set(); // game version columns whose profile dropdown is open

function currentVersionKey() {
  const v = gameVersions.find((x) => x.current);
  if (v) return { key: v.version || "", label: versionLabel(v) };
  const raw = currentGame?.install?.exe_product_version || currentGame?.install?.exe_file_version || "";
  return { key: raw, label: raw || "unknown" };
}

async function loadProfiles() {
  if (!currentGame) return;
  const gameId = currentGame.id;
  const list = await invoke("list_profiles", { gameId });
  if (currentGame?.id !== gameId) return;
  profiles = list;
  if (typeof profilePick === "number" && !profiles.some((p) => p.id === profilePick)) { profilePick = null; profileRenaming = false; }
  renderProfiles();
}

function profileSummary(p) {
  const parts = [`${p.mod_count} mod${p.mod_count === 1 ? "" : "s"}`];
  if (p.active) parts.push("active");
  else {
    if (p.to_enable) parts.push(`${p.to_enable} to enable`);
    if (p.to_disable) parts.push(`${p.to_disable} to disable`);
  }
  if (p.missing.length) parts.push(`${p.missing.length} not installed`);
  return parts.join(" · ");
}

function renderProfiles() {
  const ribbon = $("#version-ribbon");
  const box = $("#version-profiles");
  if (!currentGame) { ribbon.classList.add("hidden"); box.classList.add("hidden"); return; }
  const cur = currentVersionKey();
  const enabled = mods.filter((m) => m.status === "installed").length;
  if (typeof profilePick === "number" && !profiles.some((p) => p.id === profilePick)) profilePick = null;

  // One column per game version (a hotfix with the same version number
  // shares the newest column), oldest first; profiles filed under a version
  // the history doesn't know go in a last column.
  const cols = [];
  for (const v of gameVersions) {
    const key = v.version || "";
    const at = cols.findIndex((c) => c.key === key);
    if (at >= 0) cols.splice(at, 1);
    cols.push({ key, label: versionLabel(v), current: !!v.current, count: v.mod_ids.length });
  }
  if (!cols.some((c) => c.current)) cols.push({ key: cur.key, label: cur.label, current: true, count: mods.length });
  const known = new Set(cols.map((c) => c.key));
  const orphans = profiles.filter((p) => !known.has(p.game_version));
  if (orphans.length) cols.push({ key: null, label: "Other game versions", current: false });

  const pick = (id) => { profilePick = profilePick === id ? null : id; profileRenaming = false; renderProfiles(); };
  const profileChip = (p) => el("button", {
    class: `chip${p.id === profilePick ? " active" : ""}${p.active ? " applied" : ""}`,
    "aria-pressed": String(p.id === profilePick),
    title: [`Saved ${p.updated_at.slice(0, 16)}`, p.collection ? `Modpack profile: Nexus collection ${p.collection}` : null, p.game_version && p.game_version !== cur.key ? `Saved under game version ${p.game_version}` : null,
      p.missing.length ? `Not installed: ${p.missing.join(", ")}` : null].filter(Boolean).join("\n"),
    onclick: () => pick(p.id),
  }, el("b", {}, p.name), el("span", { class: "muted small" }, profileSummary(p)));

  // Each column: the game version on top, the stock tag under it (current
  // version only), then the version's profiles in a dropdown.
  const columns = cols.map((c, i) => {
    const mine = c.key === null ? orphans : profiles.filter((p) => p.game_version === c.key);
    const colKey = c.key ?? "~other";
    const open = profilesOpen.has(colKey) || mine.some((p) => p.id === profilePick) || (c.current && profilePick === "new");
    const applied = mine.find((p) => p.active);
    const title = el("div", { class: "ver-title" },
      el("b", {}, c.label), c.current ? el("span", { class: "ver-now" }, "current") : null,
      c.count != null ? el("span", { class: "muted small" }, `${c.count} installed`) : null);
    const stock = c.current ? el("button", {
      class: `chip stock-chip${profilePick === "stock" ? " active" : ""}${enabled === 0 ? " applied" : ""}`,
      "aria-pressed": String(profilePick === "stock"),
      title: "Stock game: every installed mod disabled",
      onclick: () => pick("stock"),
    }, el("b", {}, "Stock"), el("span", { class: "small" }, enabled === 0 ? "active" : "no mods")) : null;
    const toggle = (mine.length || c.current) ? el("button", {
      class: `chip profiles-toggle${open ? " open" : ""}${applied ? " applied" : ""}`,
      "aria-expanded": String(open),
      title: open ? "Hide the profiles of this game version" : "Show the profiles of this game version",
      onclick: () => {
        if (open) { profilesOpen.delete(colKey); if (mine.some((p) => p.id === profilePick) || (c.current && profilePick === "new")) profilePick = null; }
        else profilesOpen.add(colKey);
        renderProfiles();
      },
    }, el("b", {}, applied ? applied.name : `Profiles (${mine.length})`),
    el("span", { class: "muted small" }, applied ? `active profile · ${mine.length} saved` : mine.length ? "none active" : "none saved"),
    el("span", { class: "caret", "aria-hidden": "true" }, open ? "▴" : "▾")) : null;
    return [i ? el("span", { class: "ribbon-arrow", "aria-hidden": "true" }, "→") : null,
      el("div", { class: `ver-col${c.current ? " current" : ""}` }, title, stock, toggle,
        open ? el("div", { class: "ver-chips" }, ...mine.map(profileChip),
          c.current ? el("button", { class: `chip new-chip${profilePick === "new" ? " active" : ""}`,
            title: "Store the mods enabled now as a profile under this game version", onclick: () => pick("new") },
          el("b", {}, "+ Profile"), el("span", { class: "muted small" }, `save ${enabled} enabled`)) : null) : null)];
  }).flat().filter(Boolean);
  ribbon.classList.remove("hidden");
  ribbon.replaceChildren(el("span", { class: "muted small ribbon-label" }, "Game version", el("br"), "and profiles"), ...columns);

  box.replaceChildren(...profileActions(cur, enabled));
  box.classList.toggle("hidden", !box.childNodes.length);
}

// The actions for the picked chip, shown under the ribbon.
function profileActions(cur, enabled) {
  if (profilePick === "stock") {
    return [el("div", { class: "card row profile-actions" },
      el("span", {}, el("b", {}, `Stock ${cur.label}`), el("span", { class: "muted small" },
        ` · disables all ${enabled} enabled mods; files not installed by CPMX2077 stay in the game folder`)),
      el("span", { class: "spacer" }),
      el("button", { class: "primary", disabled: enabled ? null : "", title: enabled ? "" : "No mods are enabled",
        onclick: (e) => busy(e.target, () => applyStock(cur, enabled)) }, "Apply"))];
  }
  if (profilePick === "new") {
    const name = el("input", { type: "text", maxlength: "60", placeholder: "Profile name", "aria-label": "New profile name" });
    const create = (e) => busy(e?.target, async () => {
      if (!name.value.trim()) throw "Enter a profile name";
      const id = await invoke("create_profile", { gameId: currentGame.id, gameVersion: cur.key, name: name.value });
      profilePick = id;
      await loadProfiles();
      toast(`Profile saved: ${enabled} enabled mods.`);
    });
    name.addEventListener("keydown", (e) => { if (e.key === "Enter") create(); });
    setTimeout(() => name.focus(), 0);
    return [el("div", { class: "card row profile-actions" }, name,
      el("button", { class: "primary", disabled: enabled ? null : "", title: enabled ? "" : "No mods are enabled", onclick: create },
        `Save ${enabled} enabled mods as profile`),
      el("button", { onclick: () => { profilePick = null; renderProfiles(); } }, "Cancel"))];
  }
  const sel = profiles.find((p) => p.id === profilePick);
  if (!sel) return [];
  const ver = { label: sel.game_version || "unknown", current: sel.game_version === cur.key };
  if (profileRenaming) {
    const input = el("input", { type: "text", value: sel.name, maxlength: "60", "aria-label": "Profile name" });
    const done = (e) => busy(e?.target, async () => {
      await invoke("rename_profile", { id: sel.id, name: input.value });
      profileRenaming = false;
      await loadProfiles();
    });
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") done();
      if (e.key === "Escape") { profileRenaming = false; renderProfiles(); }
    });
    setTimeout(() => input.focus(), 0);
    return [el("div", { class: "card row profile-actions" }, input,
      el("button", { class: "primary", onclick: done }, "Rename"),
      el("button", { onclick: () => { profileRenaming = false; renderProfiles(); } }, "Cancel"))];
  }
  return [el("div", { class: "card row profile-actions" },
    el("b", {}, sel.name),
    el("span", { class: "muted small" }, `${ver.current ? "" : `saved under ${ver.label} · `}${profileSummary(sel)}`),
    el("span", { class: "spacer" }),
    el("button", {
      class: "primary", disabled: sel.active ? "" : null,
      title: sel.active ? "The enabled mods already match this profile" : "Enable the profile's mods and disable every other installed mod",
      onclick: (e) => busy(e.target, () => applyProfile(sel, ver)),
    }, "Apply"),
    el("button", {
      title: "Replace the profile's mod list with the mods enabled now",
      onclick: (e) => busy(e.target, async () => {
        if (!(await dialog.ask(`Replace the mod list of ${sel.name} with the ${enabled} mods enabled now?`,
          { title: "Update profile", kind: "info", okLabel: "Update", cancelLabel: "Cancel" }))) return;
        await invoke("save_profile", { id: sel.id });
        await loadProfiles();
        toast(`${sel.name}: saved ${enabled} enabled mods.`);
      }),
    }, "Update from enabled mods"),
    el("button", { onclick: () => { profileRenaming = true; renderProfiles(); } }, "Rename"),
    el("button", {
      class: "danger",
      onclick: (e) => busy(e.target, async () => {
        if (!(await dialog.ask(`Delete the profile ${sel.name}? Installed mods are not changed.`,
          { title: "Delete profile", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel" }))) return;
        await invoke("delete_profile", { id: sel.id });
        profilePick = null;
        await loadProfiles();
      }),
    }, "Delete"))];
}

// Disable every mod. When the enabled mods match no profile, offer to save
// them first so they can be brought back in one click.
async function applyStock(cur, enabled) {
  const saved = profiles.some((p) => p.active);
  const lines = [`Disable all ${enabled} enabled mods (stock ${cur.label})?`,
    "Their files leave the game folder; stored copies are kept. Files not installed by CPMX2077 stay."];
  if (!saved) lines.push("", "The enabled mods match no profile. Save them as a profile first to switch back in one click.");
  if (!(await dialog.ask(lines.join("\n"), { title: "Stock game", kind: "warning", okLabel: "Disable all", cancelLabel: "Cancel" }))) return;
  if (!saved && (await dialog.ask(`Save the ${enabled} enabled mods as profile “Before stock”?`,
    { title: "Stock game", kind: "info", okLabel: "Save profile", cancelLabel: "Don't save" }))) {
    await invoke("create_profile", { gameId: currentGame.id, gameVersion: cur.key, name: uniqueProfileName("Before stock") });
  }
  try {
    const r = await invoke("apply_stock_profile", { gameId: currentGame.id });
    const extra = r.kept_files ? [`${r.kept_files} file${r.kept_files === 1 ? "" : "s"} changed after install left in the game folder.`] : [];
    bulkReport(`Stock ${cur.label}: disabled ${r.disabled.length}`, r.failed.map(([n, e]) => `${n}: ${e}`), extra);
  } finally {
    profilePick = null;
    await loadMods();
  }
}

function uniqueProfileName(base) {
  const names = new Set(profiles.map((p) => p.name.toLowerCase()));
  let n = base;
  for (let i = 2; names.has(n.toLowerCase()); i++) n = `${base} ${i}`;
  return n;
}

async function applyProfile(p, ver) {
  const plan = await invoke("profile_plan", { id: p.id });
  const byId = new Map(mods.map((m) => [m.id, m.name]));
  const lines = [`Apply profile ${p.name}?`];
  if (!ver.current) lines.push(`Saved under game version ${ver.label}; the game is now ${currentVersionKey().label}. Mods may need updates.`);
  if (plan.enable.length) lines.push(`Enable (${plan.enable.length}):`, nameList(plan.enable.map((id) => byId.get(id) || `#${id}`), 8));
  if (plan.disable.length) lines.push(`Disable (${plan.disable.length}):`, nameList(plan.disable.map((id) => byId.get(id) || `#${id}`), 8));
  if (plan.missing.length) lines.push(`Not installed, skipped (${plan.missing.length}):`, nameList(plan.missing, 8));
  if (!plan.enable.length && !plan.disable.length) throw "The enabled mods already match this profile";
  if (!(await dialog.ask(lines.join("\n"), { title: "Apply profile", kind: "info", okLabel: "Apply", cancelLabel: "Cancel" }))) return;
  try {
    const r = await invoke("apply_profile", { id: p.id });
    const extra = [];
    if (r.kept_files) extra.push(`${r.kept_files} file${r.kept_files === 1 ? "" : "s"} changed after install left in the game folder.`);
    if (r.missing.length) extra.push(`Not installed: ${r.missing.join(", ")}`);
    bulkReport(`${p.name}: enabled ${r.enabled.length}, disabled ${r.disabled.length}`, r.failed.map(([n, e]) => `${n}: ${e}`), extra);
  } finally {
    profilePick = null;
    await loadMods();
  }
}
