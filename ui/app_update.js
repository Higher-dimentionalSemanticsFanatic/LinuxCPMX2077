// CPMX2077 update notice: compares this build with the newest GitHub release
// (pre-releases included) on startup and from Settings → Check now. Notify
// only; the release page opens in the browser. Loaded after app.js and built
// from its helpers ($, invoke, busy, toast, fmtDate). Release notes are
// inserted as text, never as HTML.

let appUpdate = null;

function appUpdateLine(s) {
  if (!s.latest) return "No releases listed";
  if (s.newer) return `${s.latest.tag} available (installed v${s.current})`;
  return `Up to date: v${s.current} (newest release ${s.latest.tag})`;
}

function showAppUpdate(s) {
  appUpdate = s;
  const l = s.latest;
  $("#app-update-title").textContent = `CPMX2077 ${l.tag} available`;
  $("#app-update-meta").textContent = [l.prerelease ? "pre-release" : "release",
    l.published ? `published ${fmtDate(l.published)}` : "", `installed v${s.current}`].filter(Boolean).join(" · ");
  $("#app-update-notes").textContent = l.notes || "No release notes.";
  $("#app-update-notes").classList.add("hidden");
  $("#app-update-notes-toggle").textContent = "Show release notes";
  $("#app-update").classList.remove("hidden");
}

async function checkAppUpdate(manual) {
  if (manual) $("#app-update-status").textContent = "Checking GitHub releases…";
  let s;
  try {
    s = await invoke("app_update_check");
  } catch (e) {
    $("#app-update-status").textContent = `Check failed: ${e}`;
    if (manual) throw e;
    return;
  }
  $("#app-update-status").textContent = `${appUpdateLine(s)} · checked ${new Date().toLocaleTimeString()}`;
  if (!s.newer) return $("#app-update").classList.add("hidden");
  const prefs = await invoke("app_update_prefs");
  if (manual || prefs.dismissed !== s.latest.tag) showAppUpdate(s);
}

$("#app-update-notes-toggle").addEventListener("click", () => {
  const hidden = $("#app-update-notes").classList.toggle("hidden");
  $("#app-update-notes-toggle").textContent = hidden ? "Show release notes" : "Hide release notes";
});
$("#app-update-open").addEventListener("click", (e) =>
  busy(e.target, () => invoke("open_url", { url: appUpdate.latest.url })));
$("#app-update-dismiss").addEventListener("click", (e) => busy(e.target, async () => {
  await invoke("app_update_dismiss", { tag: appUpdate.latest.tag });
  $("#app-update").classList.add("hidden");
}));
$("#app-update-check").addEventListener("click", (e) => busy(e.target, () => checkAppUpdate(true)));
$("#app-update-startup").addEventListener("change", (e) =>
  busy(null, () => invoke("set_app_update_on_startup", { on: e.target.checked })));

window.addEventListener("DOMContentLoaded", () => busy(null, async () => {
  const prefs = await invoke("app_update_prefs");
  $("#app-version").textContent = `v${prefs.current}`;
  $("#app-update-startup").checked = prefs.on_startup;
  if (prefs.on_startup) await checkAppUpdate(false);
  else $("#app-update-status").textContent = "Not checked (startup check off)";
}));
