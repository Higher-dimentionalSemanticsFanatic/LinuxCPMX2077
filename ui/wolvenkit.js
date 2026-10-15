// WolvenKit section of the Netrunner tab: download WolvenKit from its GitHub
// releases (the CLI everywhere, the editor on Windows), show the .NET runtime
// the CLI needs, and list an archive's files with the CLI. RadioEXT uses the
// CLI to check station icons. Loaded after app.js and built from its helpers.

const WK_REPO = "https://github.com/WolvenKit/WolvenKit";
const WK_WIKI = "https://wiki.redmodding.org/wolvenkit";
let wkStatus = null;
let wkLatest = null; // { tag, url, published, cli, editor } or { error }
let wkProgress = "";
let wkListing = null; // { name, tool, files, more } or null

listen("wolvenkit-progress", (e) => {
  const { what, done, total } = e.payload;
  wkProgress = total ? `Downloading ${what}: ${fmtSize(done)} of ${fmtSize(total)}` : `Downloading ${what}`;
  const p = $("#wolvenkit-progress");
  if (p) p.textContent = wkProgress;
});

async function wkLoad() {
  wkStatus = await invoke("wolvenkit_status");
  renderWolvenKit();
  if (!wkLatest || wkLatest.error) {
    wkLatest = await invoke("wolvenkit_latest").catch((e) => ({ error: String(e) }));
    renderWolvenKit();
  }
}
window.loadWolvenKit = () => wkLoad().catch((e) => toast(String(e), true));

async function wkInstall(part) {
  wkProgress = "";
  try {
    const r = await invoke("wolvenkit_install", { part });
    toast(`${part === "cli" ? "WolvenKit CLI" : "WolvenKit editor"} ${r.version} installed${r.verified ? "; SHA-256 matches GitHub's digest" : "; GitHub published no digest for this file"}.`);
  } finally {
    wkProgress = "";
    await wkLoad();
  }
}

async function wkRemove(part, label) {
  if (!await dialog.ask(`Remove ${label} from CPMX2077's data folder?`, { title: "WolvenKit", kind: "warning" })) return;
  await invoke("wolvenkit_remove", { part });
  await wkLoad();
}

async function wkListArchive() {
  const path = await dialog.open({ title: "Choose an archive", filters: [{ name: "Game archives", extensions: ["archive"] }] });
  if (!path) return;
  const r = await invoke("wolvenkit_list_archive", { path });
  wkListing = { name: path.split(/[\\/]/).pop(), ...r };
  renderWolvenKit();
}

function wkVersionCell(inst, part) {
  if (!inst) return el("span", { class: "badge bad" }, "not installed");
  const newer = wkLatest?.tag && wkLatest.tag !== inst.version && wkLatest[part];
  return el("span", {}, `${inst.version} `,
    el("span", { class: `badge ${inst.verified ? "ok" : "bad"}`, title: `Installed ${fmtDate(inst.installed)} from ${inst.asset}` },
      inst.verified ? "SHA-256 matches GitHub" : "no GitHub digest"),
    newer ? el("span", { class: "badge" }, `${wkLatest.tag} available`) : null);
}

function wkPartButtons(inst, part, label) {
  const offered = wkLatest && !wkLatest.error ? wkLatest[part] : null;
  const newer = inst && wkLatest?.tag && wkLatest.tag !== inst.version;
  return [
    !inst || newer ? el("button", { class: inst ? null : "primary", disabled: offered ? null : "",
      title: offered ? `Download ${offered} from ${WK_REPO}/releases` : "Newest release not read from GitHub",
      onclick: (e) => busy(e.target, () => wkInstall(part)) }, inst ? "Update" : "Install") : null,
    part === "editor" && inst ? el("button", { onclick: (e) => busy(e.target, () => invoke("wolvenkit_open_editor")) }, "Open WolvenKit") : null,
    inst ? el("button", { class: "danger", onclick: (e) => busy(e.target, () => wkRemove(part, label)) }, "Remove") : null,
  ];
}

function wkRuntimeRow(s) {
  const rt = s.runtime;
  const need = rt.required ? `${rt.required.name} ${rt.required.version}` : "named in the CLI's runtimeconfig.json, checked after install";
  const found = rt.found.filter((f) => f.name === "Microsoft.NETCore.App").map((f) => f.version);
  // The editor is offered on Windows only, so this also says which system runs.
  const help = s.editor_available
    ? docLink("https://dotnet.microsoft.com/download/dotnet", [".NET runtime downloads (Microsoft)"])
    : el("span", {}, "Install the distribution's .NET runtime package (dotnet-runtime-10.0 on Ubuntu and Fedora, dotnet-runtime on Arch) or ",
      docLink("https://learn.microsoft.com/dotnet/core/install/linux-scripted-manual", ["Microsoft's install script"]), " into ~/.dotnet.");
  return el("tr", {}, el("td", {}, ".NET runtime"), el("td", {},
    s.cli ? el("span", { class: `badge ${rt.ok ? "ok" : "bad"}` }, rt.ok ? "found" : "missing") : null,
    el("div", { class: "muted small" }, `Needed: ${need}. `, rt.dotnet
      ? `dotnet: ${rt.dotnet}; runtimes: ${found.length ? found.join(", ") : "none"}.`
      : "dotnet not found (DOTNET_ROOT, PATH, ~/.dotnet, standard folders)."),
    (s.cli && !rt.ok) || !rt.dotnet ? el("div", { class: "small" }, help) : null), el("td", {}));
}

function renderWolvenKit() {
  const box = $("#wolvenkit");
  const s = wkStatus;
  if (!box || !s) return;
  const latest = !wkLatest ? "reading GitHub…"
    : wkLatest.error ? `GitHub not reachable: ${wkLatest.error}`
    : `${wkLatest.tag}${wkLatest.published ? ` (${fmtDate(wkLatest.published)})` : ""}`;
  const cliReady = s.cli && s.runtime.ok;
  const rows = [
    el("tr", {}, el("td", {}, "Newest release"), el("td", {}, latest, " ", docLink(wkLatest?.url || `${WK_REPO}/releases`, ["release page"])), el("td", {})),
    el("tr", {}, el("td", {}, "WolvenKit CLI"), el("td", {}, wkVersionCell(s.cli, "cli"),
      el("div", { class: "muted small" }, "Command-line build (WolvenKit.CLI): reads, extracts and packs archives.")),
      el("td", { class: "actions" }, ...wkPartButtons(s.cli, "cli", "the WolvenKit CLI"))),
    wkRuntimeRow(s),
    el("tr", {}, el("td", {}, "WolvenKit editor"), el("td", {},
      s.editor_available ? wkVersionCell(s.editor, "editor")
        : el("span", { class: "muted" }, "Windows program (WPF); not offered on Linux. The CLI covers what CPMX2077 uses."),
      s.editor_available ? el("div", { class: "muted small" }, "Self-contained; no .NET install needed.") : null),
      el("td", { class: "actions" }, ...(s.editor_available ? wkPartButtons(s.editor, "editor", "the WolvenKit editor") : []))),
  ];
  const listing = wkListing ? el("details", { open: "" },
    el("summary", {}, el("b", {}, wkListing.name), el("span", { class: "muted" },
      ` · ${plural(wkListing.files.length + wkListing.more, "file")} · ${wkListing.tool}`)),
    el("pre", { class: "wk-listing" }, wkListing.files.join("\n") + (wkListing.more ? `\n… ${wkListing.more} more` : ""))) : null;
  box.replaceChildren(
    el("table", { class: "req-table" }, el("tbody", {}, ...rows)),
    el("p", { id: "wolvenkit-progress", class: "muted small" }, wkProgress),
    el("div", { class: "row" },
      el("button", { disabled: cliReady ? null : "", title: cliReady ? "List the files in an .archive, named from WolvenKit's hash list or the archive's own list"
        : "Needs the WolvenKit CLI and its .NET runtime", onclick: (e) => busy(e.target, wkListArchive) }, "List archive contents…"),
      el("button", { onclick: (e) => busy(e.target, () => invoke("wolvenkit_open_folder")) }, "Open folder"),
      docLink(WK_WIKI, ["WolvenKit wiki"])),
    el("p", { class: "muted small" }, "Used by: Installed mods › RadioEXT › Check icon (reads a station's PNG icon archive back and compares it byte for byte)."),
    ...(listing ? [listing] : []));
}
