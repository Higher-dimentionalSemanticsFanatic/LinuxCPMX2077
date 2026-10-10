// Introduction tour: a short walkthrough of the window, the core frameworks
// from GitHub and the Nexus account. Opens once on first start; Settings ›
// Start tour reopens it. Each step switches to its tab, outlines one element
// and blocks clicks everywhere else, so the outlined control stays usable.

const TOUR_GUIDE = "https://github.com/EkranoplanCC/LinuxCPMX2077/blob/main/docs/getting-started.md";
const tourOnWindows = /Windows/i.test(navigator.userAgent);

let tour = null; // { i, steps, tab, source } while open
let tourWatch = null; // re-places the card while lists and panels fill in
let tourFrame = 0;

function visible(sel) {
  const e = typeof sel === "string" ? document.querySelector(sel) : sel;
  return e && e.getClientRects().length ? e : null;
}

function tourSteps() {
  const steps = [
    {
      title: "CPMX2077",
      body: [
        "Mod manager for Cyberpunk 2077. This tour covers the window layout, the core frameworks from GitHub and the Nexus account.",
        "Skip tour closes it. Settings › Start tour reopens it.",
      ],
    },
    {
      title: "Game",
      target: () => "#game-panel",
      body: [
        tourOnWindows
          ? "Cyberpunk 2077 installs found in Steam, GOG Galaxy and Epic. Every tab works on the install selected here."
          : "Cyberpunk 2077 installs found in Steam (native, Flatpak, Snap) and Heroic libraries. Every tab works on the install selected here.",
        "Install not listed: Add path… and select the folder that contains bin/x64/Cyberpunk2077.exe.",
        "Frameworks: highlighted names are present in the game folder.",
      ],
    },
    {
      title: "Tabs",
      target: () => "#sidebar nav",
      body: [
        el("ul", {},
          el("li", {}, el("b", {}, "Installed mods"), ": enable, disable, update, verify, uninstall."),
          el("li", {}, el("b", {}, "Modpacks"), ": Nexus collections, tags, mod list import and export."),
          el("li", {}, el("b", {}, "Netrunner"), ": setup problems, crashes and logs, file map, graph, compatibility."),
          el("li", {}, el("b", {}, "Get mods"), ": Nexus Mods and GitHub."),
          el("li", {}, el("b", {}, "Downloads"), ": downloaded archives per mod and version; reinstall or switch."),
          el("li", {}, el("b", {}, "Settings"), ": download location, nxm:// links, debug terminal.")),
      ],
    },
    {
      title: "Nexus account",
      tab: "nexus",
      source: "nexus",
      target: () => visible("#nexus-user") ? "#nexus-user" : "#nexus-login",
      body: () => [
        nexusUser
          ? `Connected: ${nexusUser.name} (${nexusUser.is_premium ? "Premium" : "Free"}).`
          : "Not connected. Copy the personal API key from nexusmods.com › Site preferences › API Keys, paste it into the field and click Connect.",
        "The key is verified with Nexus and stored in the system keyring, not in a file.",
        el("ul", {},
          el("li", {}, el("b", {}, "Free"), ": Get from Nexus opens the file page in the in-app Nexus window. Click Slow download there; the download is caught, checked and installed."),
          el("li", {}, el("b", {}, "Premium"), ": Download & install. No browser step.")),
      ],
    },
    {
      title: "Core frameworks from GitHub",
      tab: "nexus",
      source: "github",
      target: () => visible("#install-frameworks") ? "#install-frameworks" : "#source-switch",
      body: [
        "Source GitHub lists CET, RED4ext, redscript, ArchiveXL, TweakXL and Codeware from their official GitHub releases. Most mods require some of them.",
        "Install missing frameworks queues every framework missing from the selected game, requirements first. Each file is checked against the SHA-256 digest GitHub publishes.",
      ],
    },
  ];
  if (!tourOnWindows) steps.push({
    title: "Linux setup",
    tab: "mods",
    target: () => document.querySelector("#game-warnings li") ? "#game-warnings" : "#game-panel",
    body: [
      "Frameworks under Proton need the Visual C++ runtime (vcrun2022) in the game prefix and DLL overrides in the Steam launch options. Missing items (also HDR and folder name spellings) are listed in the Game panel, each with a fix button and, once fixed, Undo.",
      "After a framework install the fixes are offered in one dialog. Close Steam before Set launch options: Steam overwrites the setting while it runs.",
    ],
  });
  steps.push(
    {
      title: "Installed mods",
      tab: "mods",
      target: () => "#tab-mods > header",
      body: [
        "Install from archive… installs a .zip, .7z or .rar from disk. Mods from Get mods are installed on download.",
        "On switch: disables a mod without uninstalling it (files leave the game, a checked copy is kept). Uninstall: removes its files and restores what they replaced.",
        "Tick the first column to Enable, Disable or Uninstall several mods at once.",
      ],
    },
    {
      title: "Netrunner",
      target: () => '#sidebar nav button[data-tab="netrunner"]',
      body: [
        "After starting the game, open Netrunner and click Check again: setup problems, the failed startup step, crashes, mods named in log errors, known problem mods and conflicts between installed mods.",
      ],
    },
    {
      title: "Done",
      body: [
        "Step-by-step first setup, including Proton fixes and multi-mod downloads: Getting started guide.",
        "Reopen this tour: Settings › Start tour.",
      ],
      extra: () => el("button", { onclick: (e) => busy(e.target, () => invoke("open_url", { url: TOUR_GUIDE })) }, "Open getting started guide"),
    },
  );
  return steps;
}

function startTour() {
  if (tour) return;
  const active = document.querySelector("nav button.active")?.dataset.tab ?? "mods";
  tour = { i: 0, steps: tourSteps(), tab: active, source: currentSource, sideScroll: $("#sidebar").scrollTop };
  savePref("tourSeen", true);
  $("#tour").classList.remove("hidden");
  tourWatch = new MutationObserver(() => {
    if (tourFrame) return;
    tourFrame = requestAnimationFrame(() => { tourFrame = 0; if (tour) renderTourStep(false); });
  });
  for (const root of [$("#sidebar"), $("main")]) {
    tourWatch.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
  }
  document.addEventListener("keydown", tourKeys, true);
  window.addEventListener("resize", placeTour);
  showTourStep(0);
}

function endTour() {
  if (!tour) return;
  const { tab, source, sideScroll } = tour;
  tour = null;
  tourWatch?.disconnect();
  tourWatch = null;
  $("#tour").classList.add("hidden");
  document.removeEventListener("keydown", tourKeys, true);
  window.removeEventListener("resize", placeTour);
  // Back to where the user was before the tour switched tabs and sources.
  if (currentSource !== source) busy(null, () => selectSource(source, false));
  showTab(tab);
  $("#sidebar").scrollTop = sideScroll;
}

function showTourStep(i) {
  tour.i = i;
  const s = tour.steps[i];
  if (s.tab && !$(`#tab-${s.tab}`).classList.contains("active")) showTab(s.tab);
  // The GitHub list and Nexus account box fill in after a request; place the
  // card now and again once the source has loaded.
  if (s.source && currentSource !== s.source) {
    busy(null, () => selectSource(s.source)).then(() => { if (tour?.i === i) renderTourStep(false); });
  }
  renderTourStep();
}

// `focus`: false when re-rendering because the page changed under the step
// (the Nexus account box after Connect, the GitHub list after loading).
function renderTourStep(focus = true) {
  const { i, steps } = tour;
  const s = steps[i];
  const last = i === steps.length - 1;
  const body = typeof s.body === "function" ? s.body() : s.body;
  $("#tour-count").textContent = `${i + 1} / ${steps.length}`;
  $("#tour-title").textContent = s.title;
  $("#tour-body").replaceChildren(...body.map((b) => typeof b === "string" ? el("p", {}, b) : b),
    s.extra ? el("div", { class: "row" }, s.extra()) : "");
  $("#tour-back").classList.toggle("hidden", i === 0);
  $("#tour-skip").classList.toggle("hidden", last);
  $("#tour-next").textContent = last ? "Finish" : "Next";
  placeTour();
  if (focus) $("#tour-next").focus();
}

// Outline the step's element, dim the rest with four blockers around it, and
// put the card next to it (right, below, above, or centred if it won't fit).
function placeTour() {
  if (!tour) return;
  const s = tour.steps[tour.i];
  const target = s.target ? visible(s.target()) : null;
  const card = $("#tour-card");
  const W = window.innerWidth, H = window.innerHeight, pad = 6, gap = 14;
  const [top, right, bottom, left] = ["top", "right", "bottom", "left"].map((k) => $(`#tour .tour-shade.${k}`));
  const spot = $("#tour-spot");
  if (!target) {
    for (const b of [top, right, bottom, left]) b.style.cssText = "";
    top.style.cssText = `top:0;left:0;width:${W}px;height:${H}px`;
    spot.classList.add("hidden");
    card.style.left = `${Math.max(16, (W - card.offsetWidth) / 2)}px`;
    card.style.top = `${Math.max(16, (H - card.offsetHeight) / 2)}px`;
    return;
  }
  target.scrollIntoView({ block: "nearest" });
  const r = target.getBoundingClientRect();
  const x0 = Math.max(0, r.left - pad), y0 = Math.max(0, r.top - pad);
  const x1 = Math.min(W, r.right + pad), y1 = Math.min(H, r.bottom + pad);
  top.style.cssText = `top:0;left:0;width:${W}px;height:${y0}px`;
  bottom.style.cssText = `top:${y1}px;left:0;width:${W}px;height:${H - y1}px`;
  left.style.cssText = `top:${y0}px;left:0;width:${x0}px;height:${y1 - y0}px`;
  right.style.cssText = `top:${y0}px;left:${x1}px;width:${W - x1}px;height:${y1 - y0}px`;
  spot.classList.remove("hidden");
  spot.style.cssText = `top:${y0}px;left:${x0}px;width:${x1 - x0}px;height:${y1 - y0}px`;

  const cw = card.offsetWidth, ch = card.offsetHeight;
  const clampY = (y) => Math.min(Math.max(16, y), H - ch - 16);
  const clampX = (x) => Math.min(Math.max(16, x), W - cw - 16);
  let x, y;
  if (x1 + gap + cw <= W - 16) { x = x1 + gap; y = clampY(y0); }
  else if (y1 + gap + ch <= H - 16) { x = clampX(x0); y = y1 + gap; }
  else if (y0 - gap - ch >= 16) { x = clampX(x0); y = y0 - gap - ch; }
  else { x = clampX((W - cw) / 2); y = clampY((H - ch) / 2); }
  card.style.left = `${x}px`;
  card.style.top = `${y}px`;
}

function tourKeys(e) {
  if (!tour) return;
  if (e.key === "Escape") { e.preventDefault(); endTour(); }
  else if (e.key === "ArrowRight" && !e.target.matches("input, select, textarea")) { e.preventDefault(); tourNext(); }
  else if (e.key === "ArrowLeft" && !e.target.matches("input, select, textarea") && tour.i > 0) { e.preventDefault(); showTourStep(tour.i - 1); }
}

function tourNext() {
  if (tour.i === tour.steps.length - 1) endTour();
  else showTourStep(tour.i + 1);
}

$("#tour-next").addEventListener("click", tourNext);
$("#tour-back").addEventListener("click", () => showTourStep(tour.i - 1));
$("#tour-skip").addEventListener("click", endTour);
$("#tour-start").addEventListener("click", startTour);

// Called by app.js at start-up, once the game and the Nexus account are read.
function firstRunTour() {
  if (!loadPref("tourSeen", false)) startTour();
}
