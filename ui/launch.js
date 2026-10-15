// Launch options in the Netrunner tab: the game's Steam launch options with a
// curly bracket under each part (variable, wrapper program, %command%, game
// argument). Selecting a part shows what it does and which Game panel item
// adds it (`launch_options_explain`). The line can be edited or pasted to
// check it; nothing here writes to Steam.
(() => {
  const SVG = "http://www.w3.org/2000/svg";
  let data = null; // last launch_options_explain result
  let selected = 0;
  let edited = false;
  let gameId = null;

  async function load(line = null) {
    if (!currentGame) return;
    const id = currentGame.id;
    const r = await invoke("launch_options_explain", { gameId: id, line });
    if (currentGame?.id !== id) return;
    data = r;
    if (gameId !== id || line === null) { selected = 0; edited = false; }
    gameId = id;
    if (line === null) $("#launch-line").value = r.line;
    selected = Math.min(selected, Math.max(0, r.parts.length - 1));
    render();
  }

  function source() {
    const out = [];
    if (edited) out.push("Edited line, not written to Steam. Current line restores Steam's.");
    else if (!data.steam) out.push("Not a Steam install: no Steam launch options read. Paste the launcher's environment variables and game arguments to check them.");
    else if (!data.line.trim()) out.push("Steam launch options: none set for Cyberpunk 2077.");
    else out.push("Steam launch options, read from Steam's localconfig.vdf.");
    if (data.windows) out.push("Windows: mod DLLs load from bin/x64 without overrides; Wine and Proton settings have no effect.");
    return out.join(" ");
  }

  function render() {
    $("#launch-source").textContent = source();
    $("#launch-current").disabled = !edited;
    const box = $("#launch-parts");
    box.replaceChildren(...data.parts.map(partEl));
    if (!data.parts.length) box.append(el("p", { class: "muted" }, "Empty line: the game starts with Steam's default command."));
    drawBraces();
    renderNote();
  }

  function partEl(p, i) {
    const cls = ["launch-part", `k-${p.kind}`];
    if (!p.known) cls.push("unknown");
    if (p.warnings.length) cls.push("warn");
    if (i === selected) cls.push("selected");
    const svg = document.createElementNS(SVG, "svg");
    svg.classList.add("launch-brace");
    svg.setAttribute("aria-hidden", "true");
    svg.append(document.createElementNS(SVG, "path"));
    return el("button", {
      class: cls.join(" "), role: "option", "aria-selected": String(i === selected), "data-i": i,
      title: p.label,
      onclick: () => choose(i),
      onkeydown: (e) => {
        const step = { ArrowRight: 1, ArrowLeft: -1, Home: -Infinity, End: Infinity }[e.key];
        if (step === undefined) return;
        e.preventDefault();
        choose(Math.max(0, Math.min(data.parts.length - 1, step === Infinity ? data.parts.length - 1 : step === -Infinity ? 0 : selected + step)), true);
      },
    },
    el("span", { class: "launch-text mono" }, p.text),
    svg,
    el("span", { class: "launch-label" }, p.warnings.length ? `! ${p.label}` : p.label));
  }

  function choose(i, focus = false) {
    selected = i;
    document.querySelectorAll("#launch-parts .launch-part").forEach((b, j) => {
      b.classList.toggle("selected", j === i);
      b.setAttribute("aria-selected", String(j === i));
      if (j === i && focus) b.focus();
    });
    renderNote();
  }

  // Horizontal curly bracket as wide as the part's text, point down at the
  // middle. Drawn from measured widths, so redrawn on resize.
  function drawBraces() {
    document.querySelectorAll("#launch-parts .launch-part").forEach((b) => {
      const w = Math.max(8, b.querySelector(".launch-text").offsetWidth);
      const h = 14, y = h / 2, r = Math.min(7, w / 4), m = w / 2, l = 1, rt = w - 1;
      const svg = b.querySelector("svg");
      svg.setAttribute("width", w);
      svg.setAttribute("height", h);
      svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
      svg.firstChild.setAttribute("d",
        `M${l},1 Q${l},${y} ${l + r},${y} L${m - r},${y} Q${m},${y} ${m},${h - 1} Q${m},${y} ${m + r},${y} L${rt - r},${y} Q${rt},${y} ${rt},1`);
    });
    placeNotch();
  }

  function placeNotch() {
    const b = document.querySelector("#launch-parts .launch-part.selected");
    const note = $("#launch-note");
    if (!b) return;
    const x = b.offsetLeft + b.offsetWidth / 2;
    note.style.setProperty("--x", `${Math.max(12, x)}px`);
  }

  function renderNote() {
    const note = $("#launch-note");
    const p = data?.parts[selected];
    note.classList.toggle("hidden", !p);
    if (!p) return;
    const items = p.items.length
      ? el("table", { class: "launch-items" }, el("tbody", {}, p.items.map((it) =>
        el("tr", {}, el("td", { class: "mono" }, it.name), el("td", {}, it.detail)))))
      : null;
    note.replaceChildren(...[
      el("h4", {}, p.label, " ", el("span", { class: "mono muted" }, p.text)),
      el("p", {}, p.detail),
      items,
      ...p.warnings.map((w) => el("p", { class: "launch-warning" }, w)),
      el("p", { class: "muted small" }, p.added_by ? `Added by: ${p.added_by}` : "Not added by CPMX2077."),
    ].filter(Boolean));
    placeNotch();
  }

  let timer;
  $("#launch-line").addEventListener("input", () => {
    edited = true;
    clearTimeout(timer);
    timer = setTimeout(() => load($("#launch-line").value).catch((e) => toast(String(e), true)), 200);
  });
  $("#launch-current").addEventListener("click", (e) => busy(e.target, () => load()));
  new ResizeObserver(() => { if (data) drawBraces(); }).observe($("#launch-parts"));

  // Reloaded with the other checks; an edited line is kept for the same game.
  window.loadLaunchExplain = () => {
    if (edited && gameId === currentGame?.id) return Promise.resolve();
    return load().catch((e) => {
      $("#launch-source").textContent = String(e);
      $("#launch-parts").replaceChildren();
      $("#launch-note").classList.add("hidden");
    });
  };
})();
