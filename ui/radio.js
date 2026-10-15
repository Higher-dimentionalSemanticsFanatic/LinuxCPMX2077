// RadioEXT section of Installed mods: radioExt's stations in the game,
// and an editor to create stations from the user's music files or change the
// ones made here. A station made here is a tracked mod, so Enable/Disable and
// Delete reuse the mod table's actions. A station icon can be made from a PNG
// (built in the backend, checked on demand with the WolvenKit CLI from the
// Netrunner tab). Loaded after app.js and built from its helpers.

const RADIO_DIR = "bin/x64/plugins/cyber_engine_tweaks/mods/radioExt/radios";
const RADIO_REPO = "justarandomguyintheinternet/CP77_radioExt";
const RADIO_EXTS = ["mp3", "mp2", "flac", "ogg", "wav", "wax", "wma", "opus", "aiff", "aif", "aifc"];

let radioStatus = null;
// The station being edited: { folder, meta, songs: [{ name, keep, path, size, needsRecent }], fixedOrder,
// iconMode: "record" | "png" | "atlas", iconPng, iconPreview, iconArchive }.
let radioEdit = null;
let radioTagNames = loadPref("radioTagNames", true);

async function loadRadio() {
  const box = $("#radio");
  if (!currentGame) return box.classList.add("hidden");
  const gameId = currentGame.id;
  const s = await invoke("radio_status", { gameId });
  if (currentGame?.id !== gameId) return;
  radioStatus = s;
  box.classList.remove("hidden");
  renderRadio();
}


function radioNewMeta() {
  const used = new Set((radioStatus?.stations || []).map((s) => s.metadata?.fm));
  let fm = 100.1;
  while (used.has(fm)) fm = Math.round((fm + 0.2) * 10) / 10;
  return { displayName: "", fm, volume: 1.0, icon: "UIIcon.alcohol_absynth",
    customIcon: { inkAtlasPath: "", inkAtlasPart: "", useCustom: false },
    streamInfo: { streamURL: "", isStream: false }, order: [] };
}

function radioStartEdit(st) {
  if (!st) {
    radioEdit = { folder: null, meta: radioNewMeta(), songs: [], fixedOrder: false, iconMode: "record", iconArchive: null };
  } else {
    const meta = st.metadata ? structuredClone(st.metadata) : { ...radioNewMeta(), displayName: st.folder };
    const order = meta.order || [];
    const songs = [...st.songs].sort((a, b) => {
      const [x, y] = [order.indexOf(a.file), order.indexOf(b.file)];
      return (x < 0) - (y < 0) || x - y || a.file.localeCompare(b.file, undefined, { numeric: true });
    }).map((s) => ({ name: s.file, keep: s.file, path: null, size: s.size }));
    const iconMode = st.icon_archive ? "png" : meta.customIcon?.useCustom ? "atlas" : "record";
    radioEdit = { folder: st.folder, meta, songs, fixedOrder: order.length > 0, iconMode, iconArchive: st.icon_archive };
  }
  renderRadio();
  $("#radio-editor")?.scrollIntoView({ block: "start" });
}

// A song name not used yet in the station being edited.
function radioFreeName(name) {
  const taken = new Set(radioEdit.songs.map((s) => s.name.toLowerCase()));
  if (!taken.has(name.toLowerCase())) return name;
  const dot = name.lastIndexOf(".");
  const [stem, ext] = [name.slice(0, dot), name.slice(dot)];
  for (let n = 2; ; n++) if (!taken.has(`${stem} (${n})${ext}`.toLowerCase())) return `${stem} (${n})${ext}`;
}

async function radioAddSongs() {
  const picked = await dialog.open({ multiple: true, title: "Choose songs",
    filters: [{ name: "Audio radioExt plays", extensions: RADIO_EXTS }] });
  const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
  if (!paths.length) return;
  const infos = await invoke("radio_song_info", { paths });
  const bad = [];
  for (const i of infos) {
    if (i.error) { bad.push(i.error); continue; }
    radioEdit.songs.push({ name: radioFreeName(radioTagNames ? i.name : i.plain_name), keep: null, path: i.path, size: i.size, needsRecent: i.needs_recent });
  }
  renderRadio();
  if (bad.length) toast(`Not added:\n${bad.join("\n")}`, true);
}

async function radioPickIcon() {
  const path = await dialog.open({ title: "Choose the station icon", filters: [{ name: "PNG image", extensions: ["png"] }] });
  if (!path) return;
  const p = await invoke("radio_icon_preview", { path });
  Object.assign(radioEdit, { iconPng: path, iconPreview: p.data_url, iconSize: `${p.width}×${p.height}` });
  renderRadio();
}

async function radioCheckIcon(st) {
  const c = await invoke("radio_icon_check", { gameId: currentGame.id, folder: st.folder });
  const lines = c.files.map((f) => `${f.path.split("\\").pop()}: ${f.class || "not read"}${f.detail ? ` ${f.detail}` : ""}, ${f.same_bytes ? "bytes identical" : "bytes differ"}`);
  toast(`${c.tool}: ${c.ok ? "icon archive read back" : "icon archive check failed"}\n${lines.join("\n")}`, !c.ok);
}

async function radioSave() {
  const d = radioEdit;
  const meta = structuredClone(d.meta);
  let iconPng = null;
  if (d.iconMode === "record") meta.customIcon.useCustom = false;
  else if (d.iconMode === "atlas") meta.customIcon.useCustom = true;
  else if (d.iconPng) { iconPng = d.iconPng; meta.customIcon.useCustom = false; }
  else if (!d.iconArchive) throw "Choose a PNG for the icon, or pick another icon source.";
  const r = await invoke("radio_save", { gameId: currentGame.id, draft: {
    folder: d.folder, metadata: meta, fixed_order: d.fixedOrder, icon_png: iconPng,
    songs: d.songs.map((s) => ({ name: s.name, keep: s.keep, path: s.path })),
  } });
  radioEdit = null;
  reportInstall(r, "radioExt reads stations at game start.");
  loadMods();
}

// The FM number at the start of a station name follows the FM field.
function radioSyncName(fm) {
  const m = radioEdit.meta;
  if (/^\d+(\.\d+)?(\s|$)/.test(m.displayName)) m.displayName = m.displayName.replace(/^\d+(\.\d+)?/, String(fm));
}

function radioField(label, input, hint) {
  return el("label", { class: "radio-field" }, el("span", {}, label), input, hint ? el("span", { class: "muted small" }, hint) : null);
}

function radioEditor() {
  const d = radioEdit;
  const m = d.meta;
  const input = (value, set, props = {}) => el("input", { value: value ?? "", ...props,
    oninput: (e) => set(e.target.value) });
  const check = (on, set, label, title) => el("label", { class: "check", title },
    el("input", { type: "checkbox", ...(on ? { checked: "" } : {}), onchange: (e) => { set(e.target.checked); renderRadio(); } }), ` ${label}`);
  const name = input(m.displayName, (v) => { m.displayName = v; }, { id: "radio-name", placeholder: `${m.fm} Station name`, maxlength: "120" });
  const fm = input(m.fm, (v) => {
    const n = parseFloat(v);
    if (Number.isFinite(n)) { m.fm = n; radioSyncName(n); name.value = m.displayName; }
  }, { type: "number", min: "0.1", max: "999.9", step: "0.1" });

  const move = (i, by) => {
    const j = i + by;
    if (j < 0 || j >= d.songs.length) return;
    [d.songs[i], d.songs[j]] = [d.songs[j], d.songs[i]];
    renderRadio();
  };
  const songRows = d.songs.map((s, i) => el("tr", {},
    el("td", { class: "muted" }, d.fixedOrder ? String(i + 1) : ""),
    el("td", {}, el("input", { value: s.name, class: "radio-song-name", title: "File name in the station; radioExt shows it as the song name",
      onchange: (e) => { s.name = e.target.value.trim(); } }),
      s.path ? el("div", { class: "muted small mono" }, s.path) : null),
    el("td", {}, fmtSize(s.size), s.path ? el("span", { class: "badge" }, "new") : null,
      s.needsRecent ? el("span", { class: "badge bad", title: "Listed by radioExt's current source; older radioExt versions skip this format" }, "recent radioExt only") : null),
    el("td", { class: "actions" },
      d.fixedOrder ? el("button", { title: "Play earlier", disabled: i ? null : "", onclick: () => move(i, -1) }, "↑") : null,
      d.fixedOrder ? el("button", { title: "Play later", disabled: i < d.songs.length - 1 ? null : "", onclick: () => move(i, 1) }, "↓") : null,
      el("button", { class: "danger", title: s.path ? "Leave this file out" : "Remove this song from the station (the stored copy is deleted on save)",
        onclick: () => { d.songs.splice(i, 1); renderRadio(); } }, "Remove"))));

  return el("div", { id: "radio-editor", class: "card radio-editor" },
    el("h3", {}, d.folder ? `Edit station: ${d.folder}` : "New station"),
    d.folder && !radioStatus.stations.find((s) => s.folder === d.folder)?.mod_id
      ? el("p", { class: "small" }, "Folder not tracked by any mod. Save installs it as a tracked station: its files are moved into CPMX2077's stored copy and put back, and Delete removes them.") : null,
    el("div", { class: "radio-grid" },
      radioField("Station name", name, "Shown in game. A leading FM number follows the FM field."),
      radioField("FM", fm, "Sort position in the station list."),
      radioField("Volume", input(m.volume, (v) => { const n = parseFloat(v); if (Number.isFinite(n)) m.volume = n; },
        { type: "number", min: "0.05", max: "10", step: "0.05" }), "1.0 = unchanged; match vanilla stations."),
      radioField("Icon source", el("div", { class: "segmented", role: "tablist", "aria-label": "Icon source" },
        ...[["record", "Game icon"], ["png", "PNG image"], ["atlas", "Mod .inkatlas"]].map(([k, label]) =>
          el("button", { class: d.iconMode === k ? "active" : null, role: "tab", onclick: () => { d.iconMode = k; renderRadio(); } }, label))))),
    radioIconEditor(d, input),
    check(m.streamInfo.isStream, (v) => { m.streamInfo.isStream = v; }, "Web stream", "Play an internet audio stream instead of song files"),
    m.streamInfo.isStream ? radioField("Stream URL", input(m.streamInfo.streamURL, (v) => { m.streamInfo.streamURL = v; },
      { type: "url", placeholder: "https://…/stream.mp3" }), "Direct audio stream (opens the browser's audio player). Song files are ignored while this is on.") : null,
    el("h3", {}, `Songs (${d.songs.length})`),
    el("p", { class: "muted small" }, `File names are the song names in game. Accepted: ${RADIO_EXTS.join(", ")}; each file's content is checked against its extension. Files are copied, the originals stay where they are.`),
    el("div", { class: "row" },
      el("button", { onclick: (e) => busy(e.target, radioAddSongs) }, "Add songs…"),
      check(radioTagNames, (v) => { radioTagNames = v; savePref("radioTagNames", v); }, "Name from tags",
        "Name added files \"Artist - Title\" from their ID3, FLAC or Ogg tags; files without tags keep their name"),
      check(d.fixedOrder, (v) => { d.fixedOrder = v; }, "Play in this order", "Write the list order as radioExt's play order; otherwise songs play shuffled")),
    d.songs.length ? el("table", { class: "req-table" }, el("tbody", {}, ...songRows))
      : el("p", { class: "muted small" }, m.streamInfo.isStream ? "No songs; the stream plays." : "No songs yet."),
    el("div", { class: "row" },
      el("button", { class: "primary", onclick: (e) => busy(e.target, radioSave) }, d.folder ? "Save station" : "Create station"),
      el("button", { onclick: () => { radioEdit = null; renderRadio(); } }, "Cancel")));
}

function radioIconEditor(d, input) {
  const m = d.meta;
  if (d.iconMode === "record") {
    return el("div", { class: "radio-grid" }, radioField("UIIcon record", input(m.icon, (v) => { m.icon = v; }, { list: "radio-icons" }),
      "A TweakDB UIIcon record. Suggestions: icons of the listed stations."),
      el("datalist", { id: "radio-icons" }, ...(radioStatus?.icons || []).map((i) => el("option", { value: i }))));
  }
  if (d.iconMode === "atlas") {
    return el("div", { class: "radio-grid" },
      radioField("inkatlas path", input(m.customIcon.inkAtlasPath, (v) => { m.customIcon.inkAtlasPath = v; },
        { placeholder: "base\\gameplay\\gui\\...\\atlas.inkatlas" }), "Resource path inside an installed archive mod, with backslashes."),
      radioField("inkatlas part", input(m.customIcon.inkAtlasPart, (v) => { m.customIcon.inkAtlasPart = v; }, { placeholder: "part_name" })));
  }
  const current = d.iconPng ? `${d.iconPng.split(/[\\/]/).pop()} (${d.iconSize}), scaled to 256×256`
    : d.iconArchive ? `current: ${d.iconArchive}` : "no PNG chosen";
  return el("div", { class: "radio-icon-row" },
    d.iconPreview ? el("img", { class: "radio-icon-preview", src: d.iconPreview, alt: "Icon preview" }) : null,
    el("div", {},
      el("div", { class: "row" }, el("button", { onclick: (e) => busy(e.target, radioPickIcon) }, d.iconPng || d.iconArchive ? "Change PNG…" : "Choose PNG…"),
        el("span", { class: "muted small" }, current)),
      el("p", { class: "muted small" }, "Centred on a transparent square and scaled to 256×256. Saved as a texture and atlas in ",
        el("span", { class: "mono" }, "archive/pc/mod/cpmx2077_radio_<station>.archive"),
        ", installed and removed with the station. In-game display not yet confirmed; Check icon reads the archive back with the WolvenKit CLI.")));
}

function radioRow(st) {
  const meta = st.metadata;
  const m = st.mod_id ? mods.find((x) => x.id === st.mod_id) : null;
  const source = st.managed ? "made in CPMX2077" : st.mod_name ? `from ${st.mod_name}` : "not tracked";
  const icon = !meta ? "" : st.icon_archive ? "PNG icon" : meta.customIcon?.useCustom ? `${meta.customIcon.inkAtlasPart} (custom)` : meta.icon;
  return el("tr", {},
    el("td", {}, meta ? Number(meta.fm).toFixed(1) : "—"),
    el("td", {}, meta?.displayName || st.folder,
      el("div", { class: "muted small" }, `${st.folder} · ${source}`),
      st.problem ? el("div", {}, el("span", { class: "badge bad" }, st.problem)) : null,
      ...st.warnings.map((w) => el("div", {}, el("span", { class: "badge bad" }, w)))),
    el("td", {}, meta?.streamInfo?.isStream ? el("span", { class: "badge" }, "stream") : plural(st.songs.length, "song"),
      meta?.order?.length ? el("div", { class: "muted small" }, "fixed order") : null),
    el("td", { class: "mono small" }, icon),
    el("td", {}, el("span", { class: `badge ${st.enabled ? "ok" : "bad"}` }, st.enabled ? "enabled" : "disabled")),
    el("td", { class: "actions" },
      st.managed || !st.mod_id ? el("button", { title: st.managed ? "Change name, icon, songs or order" : "Edit and save as a tracked station",
        onclick: () => radioStartEdit(st) }, "Edit") : null,
      st.icon_archive && st.enabled ? el("button", { title: "Read the icon archive back with the WolvenKit CLI (Netrunner › WolvenKit) and compare it byte for byte",
        onclick: (e) => busy(e.target, () => radioCheckIcon(st)) }, "Check icon") : null,
      st.enabled ? el("button", { title: "Open the station folder",
        onclick: (e) => busy(e.target, () => invoke("open_game_folder", { gameId: currentGame.id, path: `${RADIO_DIR}/${st.folder}` })) }, "Open folder") : null,
      st.managed && m ? el("button", { onclick: (e) => busy(e.target, () => setEnabled(m, !st.enabled)).finally(loadMods) }, st.enabled ? "Disable" : "Enable") : null,
      st.managed && m ? el("button", { class: "danger", onclick: (e) => busy(e.target, () => uninstall(m)) }, "Delete") : null));
}

function renderRadio() {
  const s = radioStatus;
  if (!s) return;
  const ready = s.installed && s.plugin;
  const state = el("table", { class: "req-table" }, el("tbody", {},
    el("tr", {}, el("td", {}, "radioExt (CET part)"), el("td", {},
      el("span", { class: `badge ${s.installed ? "ok" : "bad"}` }, s.installed ? "installed" : "not found"),
      s.mod_name ? el("span", { class: "muted small" }, ` from ${s.mod_name}`) : null)),
    el("tr", {}, el("td", {}, "radioExt (RED4ext part)"), el("td", {},
      el("span", { class: `badge ${s.plugin ? "ok" : "bad"}` }, s.plugin ? "installed" : "not found"),
      s.installed && !s.plugin ? el("span", { class: "muted small" }, " red4ext/plugins/RadioExt/RadioExt.dll missing; stations don't play") : null))));
  const stations = s.stations.length
    ? el("table", { class: "req-table radio-stations" },
      el("thead", {}, el("tr", {}, ...["FM", "Station", "Songs", "Icon", "State", ""].map((h) => el("th", {}, h)))),
      el("tbody", {}, ...s.stations.map(radioRow)))
    : el("p", { class: "muted small" }, `No stations in ${RADIO_DIR}.`);
  const details = el("details", { open: "" },
    el("summary", {}, el("b", {}, "radioExt stations"), el("span", { class: "muted" }, ` · ${plural(s.stations.length, "station")}`)),
    el("p", { class: "muted small" },
      "Custom radio stations for radioExt (needs CET and RED4ext). Each station is a folder in ", el("span", { class: "mono" }, RADIO_DIR),
      " with a metadata.json and its song files. Stations made here are tracked mods: Disable, Delete, Verify and profiles apply. Stations installed by another mod are changed through that mod."),
    state,
    el("div", { class: "row" },
      !ready ? el("button", { class: "primary", title: `Queue radioExt's newest release from GitHub (${RADIO_REPO})`,
        onclick: (e) => busy(e.target, () => enqueue([{ kind: "source", name: "radioExt", source: "github", ref: RADIO_REPO }])) },
      s.installed ? "Reinstall radioExt" : "Install radioExt") : null,
      el("button", { class: ready ? "primary" : null, onclick: () => radioStartEdit(null) }, "New station"),
      docLink(`https://github.com/${RADIO_REPO}#creating-a-new-station`, ["radioExt station format"])),
    stations,
    radioEdit ? radioEditor() : null);
  $("#radio").replaceChildren(details);
}
