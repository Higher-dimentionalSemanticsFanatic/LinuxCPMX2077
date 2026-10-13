// OptiScaler page in Installed mods: install OptiScaler from its GitHub
// releases (optionally with AMD's FSR 4.1.1 upscaler DLL), and pick the
// FSR 4.1.1 shader overrides vkd3d-proton loads through the launch options.
// OptiScaler is a tracked mod, so Disable and Uninstall reuse the mod table's
// actions; the launch options are Linux setup checks (app.js). Loaded after
// app.js and built from its helpers.

let optiLatest = null;
let optiProgress = "";
let optiFsr = true;
let optiProxy = null;
const OPTI_REPO = "https://github.com/optiscaler/OptiScaler";
const FSR4_REPO = "https://github.com/zgauthier2000/fsr4-shader-performance-overrides";

listen("optiscaler-progress", (e) => {
  const { what, done, total } = e.payload;
  optiProgress = what === "shader overrides" ? `Downloading shader overrides: ${done} of ${total} files`
    : total ? `Downloading ${what}: ${fmtSize(done)} of ${fmtSize(total)}` : `Downloading ${what}`;
  const p = $("#optiscaler-progress");
  if (p) p.textContent = optiProgress;
});

async function loadOptiScaler() {
  const box = $("#optiscaler");
  if (!currentGame) return box.classList.add("hidden");
  const gameId = currentGame.id;
  const [s, o] = await Promise.all([
    invoke("optiscaler_status", { gameId }),
    invoke("fsr4_overrides", { gameId }),
  ]);
  if (currentGame?.id !== gameId) return;
  box.classList.remove("hidden");
  renderOptiScaler(s, o);
}

async function optiInstall() {
  optiProgress = "";
  try {
    const r = await invoke("optiscaler_install", { gameId: currentGame.id, proxy: optiProxy, fsr: optiFsr });
    reportInstall(r, "In game, the Insert key opens OptiScaler's overlay.");
  } finally {
    optiProgress = "";
    await loadMods();
    loadSetup().catch(() => {});
  }
}

// A Linux setup check by id, shown inline with its own buttons.
function optiCheck(id) {
  const c = setupChecks.find((x) => x.id === id);
  if (!c) return null;
  // A fix or undo reloads the setup checks, which redraws this page.
  return el("ul", { class: "warnings" }, setupItem(c));
}

function optiSummary(s) {
  const i = s.installed;
  if (i) return `${i.version || "?"} as ${i.proxy || "?"}${i.enabled ? "" : " · disabled"}`;
  if (s.unmanaged) return "not managed: OptiScaler.ini is in bin/x64 but was not installed by CPMX2077";
  return "not installed";
}

function upscalerRow(s) {
  const u = s.upscaler;
  return el("tr", {}, el("td", {}, "Upscaler DLL"), el("td", {},
    u ? [`bin/x64/amd_fidelityfx_upscaler_dx12.dll ${u.version || "?"}`, " ",
      el("span", { class: `badge ${s.upscaler_matches ? "ok" : "bad"}` }, s.upscaler_matches ? "AMD 4.1.1.2740" : "not 4.1.1.2740")]
      : el("span", { class: "muted" }, "none in bin/x64")));
}

async function fsr4Choose(variant) {
  if (variant === "lossy") {
    const go = await dialog.ask(
      "The lossy shaders change the picture: FSR 4's network runs on every other frame. Thin distant lines are dimmer while the camera pans, moving foliage is softer, and frame times alternate. Details: WARNING.txt in the repository's prebuilt-lossy folder.\n\nUse the lossy shaders?",
      { title: "FSR 4 shader overrides", kind: "warning" });
    if (!go) return;
  }
  await invoke("fsr4_overrides_choose", { gameId: currentGame.id, variant });
  await loadSetup();
}

function fsr4Section(s, o) {
  if (!o.available) return null;
  const info = o.info;
  const date = info?.date ? new Date(info.date * 1000).toISOString().slice(0, 10) : "";
  const radio = (value, label, hint) => el("label", { class: "row small" },
    el("input", { type: "radio", name: "fsr4-variant", checked: (o.choice || "off") === value ? "" : null,
      disabled: value !== "off" && !info ? "" : null,
      onchange: () => fsr4Choose(value === "off" ? null : value).catch((e) => toast(String(e), true)) }),
    el("span", {}, el("b", {}, label), hint ? el("span", { class: "muted" }, ` · ${hint}`) : null));
  return [
    el("h3", {}, "FSR 4.1.1 shader overrides"),
    el("p", { class: "muted small" },
      "Faster replacement shaders for AMD's FSR 4.1.1 from ", docLink(FSR4_REPO, ["zgauthier2000/fsr4-shader-performance-overrides"]),
      ". vkd3d-proton loads them through the VKD3D_SHADER_OVERRIDE launch option. They match AMD's upscaler DLL 4.1.1.2740 only, ",
      "for Radeon RX 7000 and RX 6000; not for RX 9000. Built from Proton's translation on an RX 7800 XT: if the picture looks wrong (darker), set Off. ",
      "Single-player only: the repository warns that anti-cheat can treat changed shaders as tampering."),
    el("table", { class: "req-table" }, el("tbody", {},
      el("tr", {}, el("td", {}, "Downloaded"), el("td", {}, info
        ? `${info.exact} exact, ${info.lossy} lossy files · commit ${info.commit.slice(0, 7)} ${date} · every file checked against its git blob id`
        : el("span", { class: "muted" }, "not yet"))),
      upscalerRow(s))),
    !s.upscaler_matches && o.choice ? el("p", { class: "small" }, el("span", { class: "badge bad" }, "no effect"),
      " The upscaler DLL in bin/x64 is not AMD's 4.1.1.2740, so no shader matches. Reinstall OptiScaler with AMD's FSR 4.1.1 upscaler DLL ticked.") : null,
    el("div", { class: "row" },
      el("button", { class: info ? null : "primary",
        title: "Read the repository's newest commit on GitHub and download every prebuilt and prebuilt-lossy .spv file, each checked against its git blob id",
        onclick: (e) => busy(e.target, async () => {
          optiProgress = "";
          try {
            const i = await invoke("fsr4_overrides_download");
            toast(`Downloaded ${i.exact} exact and ${i.lossy} lossy shader overrides (commit ${i.commit.slice(0, 7)})`);
          } finally { optiProgress = ""; loadOptiScaler(); }
        }) }, info ? "Download again" : "Download shader overrides"),
      docLink(`${FSR4_REPO}/blob/main/prebuilt-lossy/WARNING.txt`, ["Lossy: what changes"])),
    el("div", {},
      radio("off", "Off"),
      radio("exact", "Exact", "AMD's picture, byte for byte"),
      radio("lossy", "Lossy", "faster; the picture differs from AMD's")),
    optiCheck("fsr4-overrides"),
  ];
}

function renderOptiScaler(s, o) {
  const i = s.installed;
  const m = i ? mods.find((x) => x.id === i.mod_id) : null;
  const newer = i && optiLatest && !(i.version || "").startsWith(optiLatest.tag);
  const names = ["wininet.dll", "dxgi.dll"];
  if (!optiProxy || !names.includes(optiProxy)) optiProxy = i?.proxy || s.proxy_choice.Ok || names[0];
  const free = !!s.proxy_choice.Ok || !!i;
  const launch = setupChecks.find((c) => c.id === "launch-options" || c.id === "dll-overrides");
  const details = el("details", { open: "" },
    el("summary", {}, el("b", {}, "OptiScaler"), el("span", { class: "muted" }, ` · ${optiSummary(s)}`)),
    el("p", { class: "muted small" },
      "Upscaler swapper: takes the game's DLSS, XeSS or FSR 3.1 input and runs another upscaler, FSR 4 included. ",
      "Downloaded from ", docLink(`${OPTI_REPO}/releases`, ["github.com/optiscaler/OptiScaler releases"]), " only (newest stable release; ",
      "the SHA-256 is checked when GitHub publishes one). Single-player only: the OptiScaler wiki warns anti-cheat can ban for it."),
    i ? el("table", { class: "req-table" }, el("tbody", {},
      el("tr", {}, el("td", {}, "Version"), el("td", {}, i.version || "?",
        optiLatest ? el("span", { class: `badge ${newer ? "bad" : "ok"}` }, newer ? `${optiLatest.tag} available` : "latest") : null)),
      el("tr", {}, el("td", {}, "Loaded as"), el("td", {}, `bin/x64/${i.proxy || "?"}`, " ",
        el("span", { class: `badge ${i.enabled ? "ok" : "bad"}` }, i.enabled ? "enabled" : "disabled"))),
      upscalerRow(s),
      el("tr", {}, el("td", {}, "Archive SHA-256"), el("td", { class: "mono small" }, i.archive_sha256)))) : null,
    !free ? el("p", { class: "small" }, `No DLL name free: ${s.proxy_choice.Err}.`) : null,
    s.unmanaged ? el("p", { class: "small" }, "OptiScaler.ini is in bin/x64 from a copy installed outside CPMX2077. Remove that copy first; the install refuses to overwrite files it doesn't track.") : null,
    el("div", { class: "row small" },
      el("label", { title: "The system DLL name OptiScaler loads as. The OptiScaler wiki lists dxgi.dll and wininet.dll for Cyberpunk 2077; ReShade usually has dxgi.dll." },
        "Load as ", el("select", { onchange: (e) => { optiProxy = e.target.value; } },
          ...names.map((n) => el("option", { value: n, selected: n === optiProxy ? "" : null }, n))))),
    el("label", { class: "row small", title: "From AMD's FidelityFX SDK 2.3.0 on GitHub (pinned commit); checked against its git blob id and version 4.1.1.2740. Replaces the upscaler DLL in OptiScaler's archive." },
      el("input", { type: "checkbox", checked: optiFsr ? "" : null, onchange: (e) => { optiFsr = e.target.checked; } }),
      el("span", {}, "Use AMD's FSR 4.1.1 upscaler DLL (4.1.1.2740, needed for the shader overrides)")),
    el("p", { id: "optiscaler-progress", class: "muted small" }, optiProgress),
    el("div", { class: "row" },
      newer ? el("button", { class: "update", onclick: (e) => busy(e.target, optiInstall) }, `Update to ${optiLatest.tag}`) : null,
      i ? el("button", { title: "Read the newest stable OptiScaler release on GitHub",
        onclick: (e) => busy(e.target, async () => {
          optiLatest = await invoke("optiscaler_latest");
          loadOptiScaler();
        }) }, "Check for updates") : null,
      el("button", { class: i ? null : "primary", disabled: free && !s.unmanaged ? null : "",
        title: "Download the newest stable OptiScaler release from GitHub and install it into bin/x64",
        onclick: (e) => busy(e.target, optiInstall) }, i ? "Reinstall" : "Install OptiScaler"),
      m ? el("button", { onclick: (e) => busy(e.target, () => setEnabled(m, !i.enabled)).finally(loadMods) },
        i.enabled ? "Disable" : "Enable") : null,
      m ? el("button", { class: "danger", onclick: (e) => busy(e.target, () => uninstall(m)) }, "Uninstall") : null,
      docLink(`${OPTI_REPO}/wiki/Cyberpunk-2077`, ["OptiScaler wiki: Cyberpunk 2077"])),
    i && launch && launch.state !== "ok" ? optiCheck(launch.id) : null,
    i ? el("p", { class: "muted small" },
      "In game: set Settings › Graphics › Resolution scaling to FSR 3.1 or XeSS (the wiki reports noise with DLSS input and path tracing on AMD), ",
      "then press Insert and pick FSR 4 in OptiScaler's overlay. The overlay also shows the upscaler time per frame.") : null,
    ...(fsr4Section(s, o) || []));
  $("#optiscaler").replaceChildren(details);
}
