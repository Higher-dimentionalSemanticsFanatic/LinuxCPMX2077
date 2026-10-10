// Mod profiles: named lists of enabled mods, filed under a game version.
// Shown under the game version ribbon in Installed mods for the picked
// version (the current one when "All" is picked). Applying a profile enables
// its mods and disables every other installed mod.
// Loaded after app.js and built from its helpers (el, $, busy, invoke, …).
// Profile and mod names are inserted as text, never as HTML.

let profiles = [];
let profilePick = null; // id of the selected profile
let profileRenaming = false;

// The version the profiles card is for: the picked ribbon chip, else the
// current game version.
function profileVersion() {
  const picked = ribbonPick != null ? gameVersions[ribbonPick] : null;
  const v = picked || gameVersions.find((x) => x.current);
  if (v) return { key: v.version || "", label: versionLabel(v), current: !!v.current };
  const raw = currentGame?.install?.exe_product_version || currentGame?.install?.exe_file_version || "";
  return { key: raw, label: raw || "unknown", current: true };
}

async function loadProfiles() {
  if (!currentGame) return;
  const gameId = currentGame.id;
  const list = await invoke("list_profiles", { gameId });
  if (currentGame?.id !== gameId) return;
  profiles = list;
  if (!profiles.some((p) => p.id === profilePick)) { profilePick = null; profileRenaming = false; }
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
  const box = $("#version-profiles");
  if (!currentGame) { box.classList.add("hidden"); return; }
  const ver = profileVersion();
  const mine = profiles.filter((p) => p.game_version === ver.key);
  const sel = mine.find((p) => p.id === profilePick) || null;
  const enabled = mods.filter((m) => m.status === "installed").length;
  box.classList.remove("hidden");

  const chips = mine.map((p) => el("button", {
    class: `chip${p.id === profilePick ? " active" : ""}${p.active ? " applied" : ""}`,
    "aria-pressed": String(p.id === profilePick),
    title: [`Saved ${p.updated_at.slice(0, 16)}`, p.missing.length ? `Not installed: ${p.missing.join(", ")}` : null].filter(Boolean).join("\n"),
    onclick: () => { profilePick = p.id === profilePick ? null : p.id; profileRenaming = false; renderProfiles(); },
  }, el("b", {}, p.name), el("span", { class: "muted small" }, profileSummary(p))));

  let actions = null;
  if (sel && profileRenaming) {
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
    actions = el("div", { class: "row profile-actions" }, input,
      el("button", { class: "primary", onclick: done }, "Rename"),
      el("button", { onclick: () => { profileRenaming = false; renderProfiles(); } }, "Cancel"));
    setTimeout(() => input.focus(), 0);
  } else if (sel) {
    actions = el("div", { class: "row profile-actions" },
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
      }, "Delete"),
      sel.missing.length ? el("span", { class: "muted small" }, `Not installed: ${sel.missing.join(", ")}`) : null);
  }

  const name = el("input", { type: "text", maxlength: "60", placeholder: "Profile name", "aria-label": "New profile name" });
  const create = (e) => busy(e?.target, async () => {
    if (!name.value.trim()) throw "Enter a profile name";
    const id = await invoke("create_profile", { gameId: currentGame.id, gameVersion: ver.key, name: name.value });
    profilePick = id;
    await loadProfiles();
    toast(`Profile saved: ${enabled} enabled mods.`);
  });
  name.addEventListener("keydown", (e) => { if (e.key === "Enter") create(); });

  box.replaceChildren(...[
    el("div", { class: "row profile-head" },
      el("b", {}, "Mod profiles"),
      el("span", { class: "muted small" }, `game version ${ver.label}${ver.current ? " (current)" : ""}`)),
    mine.length
      ? el("div", { class: "ribbon profile-chips", role: "group", "aria-label": "Mod profiles" }, ...chips)
      : el("p", { class: "muted small" }, ver.current
        ? "No profiles for this game version. Saving one stores the list of mods enabled now."
        : "No profiles saved under this game version."),
    actions,
    ver.current ? el("div", { class: "row profile-new" }, name,
      el("button", { onclick: create, title: "Store the mods enabled now as a profile under this game version" }, `Save ${enabled} enabled mods as profile`)) : null,
  ].filter(Boolean));
}

async function applyProfile(p, ver) {
  const plan = await invoke("profile_plan", { id: p.id });
  const byId = new Map(mods.map((m) => [m.id, m.name]));
  const lines = [`Apply profile ${p.name}?`];
  if (!ver.current) {
    const cur = gameVersions.find((x) => x.current);
    lines.push(`Saved under game version ${ver.label}; the game is now ${cur ? versionLabel(cur) : "another version"}. Mods may need updates.`);
  }
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
    await loadMods();
  }
}
