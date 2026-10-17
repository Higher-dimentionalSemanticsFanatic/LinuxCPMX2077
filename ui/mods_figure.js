// Installed mods › Canvas › Figure: mods with Characters and Clothing tags
// placed on a drawn body, one slot per body part or clothing slot, with
// leader lines to the body. Opened from a Characters or Clothing column of
// the board (Figure button); ‹ Board goes back.
//
// A mod's slot comes from, in order: its files (core::figure: TweakXL item
// types, depot paths in tweaks / ArchiveXL files, and archive paths WolvenKit
// listed), words in its name, else its tag alone. The drawing is original.
(() => {
  const SVGNS = "http://www.w3.org/2000/svg";
  const sv = (tag, attrs = {}, ...kids) => { const e = document.createElementNS(SVGNS, tag); for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v); e.append(...kids); return e; };

  // [key, label, side, anchor x, y] in the drawing's 400×820 space.
  const FIG_SLOTS = [
    ["hair", "Hair", "L", 184, 46], ["eyes", "Eyes", "L", 187, 85], ["face", "Face", "L", 177, 104], ["makeup", "Makeup", "L", 194, 118],
    ["skin", "Skin", "L", 136, 196], ["tattoos", "Tattoos & scars", "L", 70, 360], ["body", "Body", "L", 150, 330],
    ["head", "Head", "R", 216, 40], ["facewear", "Face wear", "R", 214, 86], ["outer", "Outer torso", "R", 262, 200], ["inner", "Inner torso", "R", 222, 262],
    ["acc", "Accessories", "R", 331, 410], ["legs", "Legs", "R", 250, 530], ["feet", "Feet", "R", 252, 778],
    ["outfit", "Full outfits", "S"], ["preset", "Character preset", "S"], ["npc", "NPC appearance", "S"],
    ["wardrobe", "Wardrobe & transmog", "S"], ["atelier", "Atelier shop", "S"], ["armor", "Armor stats", "S"],
  ].map(([key, label, side, x, y]) => ({ key, label, side, x, y }));
  const BODY_SLOTS = FIG_SLOTS.filter((s) => s.side !== "S");
  const SLOT = new Map(FIG_SLOTS.map((s) => [s.key, s]));

  // Predefined tag → the slots it can split into (first = without a finer signal).
  const TAG_SLOTS = {
    "Face & hair": ["face", "hair", "eyes"], "Skin & tattoos": ["skin", "tattoos"], "Body": ["body"], "Makeup": ["makeup"],
    "Outfits": ["outfit", "head", "facewear", "outer", "inner", "legs", "feet"], "Accessories": ["acc", "facewear"],
    "Character preset": ["preset"], "NPC appearance": ["npc"], "Wardrobe & transmog": ["wardrobe"], "Atelier shop": ["atelier"], "Armor stats": ["armor"],
  };
  // Slot lit when the figure opens from that tag's column.
  const TAG_LIT = { "Face & hair": "hair", "Outfits": "outer" };
  const WORDS = {
    hair: ["hair", "ponytail", "braid", "beard", "brow", "mohawk", "dread", "undercut", "fringe", "bangs", "mullet"],
    eyes: ["eye", "iris", "pupil", "sclera", "contact lens"],
    face: ["face", "head morph", "cheek", "jaw", "lips shape"],
    skin: ["skin", "freckle", "mole", "pore", "complexion"],
    tattoos: ["tattoo", "scar"],
    head: ["hat", "cap", "helmet", "hood", "beanie", "bandana", "headband", "headwear"],
    facewear: ["glasses", "sunglasses", "mask", "visor", "goggles", "shades", "aviator", "eyewear"],
    outer: ["jacket", "coat", "hoodie", "vest", "blazer", "parka", "bomber", "trench", "duster", "cape", "poncho", "kimono"],
    inner: ["shirt", "tshirt", "t-shirt", "top", "tank", "bra", "blouse", "sweater", "corset", "turtleneck"],
    legs: ["pants", "jeans", "shorts", "skirt", "trousers", "leggings", "cargo"],
    feet: ["boot", "shoe", "sneaker", "heels", "sandal", "footwear"],
    acc: ["ring", "nails", "necklace", "earring", "choker", "bracelet", "piercing", "jewelry", "jewellery", "watch"],
  };
  const wordRe = Object.fromEntries(Object.entries(WORDS).map(([k, ws]) => [k, new RegExp(`(^|[^a-z])(${ws.join("|")})`, "i")]));
  const GARMENT = ["head", "facewear", "outer", "inner", "legs", "feet"];
  const VIA = { files: "FILES", name: "NAME", tag: "TAG" };
  const VIA_TITLE = { files: "its files (TweakXL item type or depot paths)", name: "words in its name", tag: "its tag only" };
  const MAX_THUMBS = 3;

  let open = false, lit = null, listOpen = null, listFilter = "", cur = {}, fit = MAX_THUMBS;
  let hints = null, hintsKey = "", wkCli = null;

  // slot key -> "files" | "name" | "tag" for one mod.
  function slotsOf(m) {
    const out = new Map();
    const files = new Set(hints?.slots?.[m.id] || []);
    for (const tag of modTags.mods[m.id] || []) {
      const cand = TAG_SLOTS[tag];
      if (!cand) continue;
      let hit = cand.filter((s) => files.has(s));
      let via = "files";
      if (!hit.length) { hit = cand.filter((s) => wordRe[s]?.test(m.name)); via = "name"; }
      if (!hit.length) { hit = [cand[0]]; via = "tag"; }
      for (const s of hit) if (!out.has(s) || via === "files") out.set(s, via);
      // A set covering three or more clothing slots is also a full outfit.
      if (tag === "Outfits" && hit.filter((s) => GARMENT.includes(s)).length >= 3) out.set("outfit", via);
    }
    return out;
  }

  async function loadHints() {
    if (!currentGame) return;
    const key = `${currentGame.id}:${mods.map((m) => `${m.id}.${m.file_count}`).join(",")}`;
    if (key === hintsKey && hints) return;
    hintsKey = key;
    hints = await invoke("figure_hints", { gameId: currentGame.id }).catch(() => null);
    if (open) renderMods();
  }

  window.figureTagSlot = (tag) => (TAG_SLOTS[tag] ? TAG_LIT[tag] || TAG_SLOTS[tag][0] : null);
  window.figureOpen = (slot) => {
    open = true; lit = slot || null; listOpen = null;
    hintsKey = ""; // re-read: files may have changed since
    renderMods();
    invoke("wolvenkit_status").then((s) => { wkCli = !!s.cli && !!s.runtime?.ok; if (open) renderMods(); }).catch(() => {});
  };
  window.figureIsOpen = () => open;
  function close() { open = false; lit = null; listOpen = null; renderMods(); }
  window.figureClose = () => { if (open) close(); };

// ---- Figure drawing (original; right half drawn, left half mirrored) ----
const HALF = "M200 34 C220 34 234 50 234 74 C234 86 233 96 231 104 C229 114 225 122 219 128 C216 131 215 134 215 138 L217 148 C230 154 254 157 272 162 C292 168 304 182 306 204 "
  + "C308 226 312 250 316 272 C320 290 324 306 328 322 C334 346 338 370 340 392 C341 398 342 401 342 404 C345 414 347 428 348 444 C349 458 348 470 345 480 "
  + "C342 487 334 489 330 483 C327 476 325 466 325 456 C322 452 319 447 318 440 C317 433 318 426 320 420 C320 412 319 404 318 398 "
  + "C316 376 312 356 306 338 C302 326 298 314 296 302 C294 282 290 262 286 244 C284 236 280 230 276 228 "
  + "C278 250 276 274 270 300 C266 318 266 336 270 356 C276 378 280 400 280 422 C280 470 274 520 268 560 C264 584 264 600 266 614 "
  + "C272 650 268 690 260 728 C258 744 258 752 262 762 C270 772 278 780 274 788 L234 790 C230 782 232 770 236 762 C238 748 238 736 236 724 "
  + "C230 690 228 650 232 616 C234 596 232 584 230 570 C224 530 216 490 208 462 L200 456";
const DETAIL = [
  "M215 138 C213 146 210 152 206 158", "M206 160 C228 164 252 166 270 166", "M202 238 C224 252 256 248 274 228", "M274 228 C282 212 290 198 298 190",
  "M276 228 C272 218 268 210 262 204", "M296 216 C304 240 306 262 304 282", "M320 330 C328 352 334 372 336 392", "M306 300 C312 316 318 330 326 340",
  "M204 266 C214 263 223 265 229 270", "M204 300 C214 297 224 299 231 303", "M204 334 C214 331 224 333 231 338", "M206 368 C214 366 222 368 228 372",
  "M238 254 C248 292 254 332 256 372", "M264 382 C248 410 224 436 206 452", "M248 468 C242 520 240 560 244 592", "M262 470 C264 500 264 530 260 560",
  "M235 600 C243 613 255 613 263 600", "M260 640 C252 670 248 700 248 728", "M240 642 C240 680 242 710 244 732",
  "M333 458 C334 466 335 474 335 484", "M339 456 C340 466 340 474 340 485", "M344 452 C345 462 345 470 344 480", "M326 446 C333 448 340 448 347 444",
  "M208 82 C214 78 221 78 226 82", "M207 74 C214 70 222 70 228 74", "M233 82 C240 82 241 98 231 103", "M201 87 L205 104 L200 108", "M210 112 C214 118 216 124 214 128", "M216 92 C220 96 224 100 226 102",
  "M200 117 C204 119 209 119 213 116", "M214 126 C210 131 205 133 200 133", "M246 776 L264 776",
];
// Bone axes for the inner glow lines (right half).
const BONES = [[[200, 150], [200, 300], [200, 450]], [[204, 168], [240, 170], [272, 176]], [[276, 180], [300, 250], [318, 320], [332, 400], [338, 462]],
  [[222, 452], [236, 520], [246, 600], [246, 680], [246, 760]]];

function figureSvg() {
  const svg = sv("svg", { viewBox: "0 -10 400 830", preserveAspectRatio: "xMidYMid meet", role: "img", "aria-label": "Hologram figure with slot points" });
  const defs = sv("defs", {},
    sv("linearGradient", { id: "fig-holo", x1: 0, y1: 0, x2: 0, y2: 1 },
      sv("stop", { offset: 0, "stop-color": "#ff8a5c", "stop-opacity": .55 }), sv("stop", { offset: .45, "stop-color": "#e8483a", "stop-opacity": .42 }), sv("stop", { offset: 1, "stop-color": "#9c2230", "stop-opacity": .38 })),
    sv("radialGradient", { id: "fig-core", cx: .5, cy: .32, r: .35 },
      sv("stop", { offset: 0, "stop-color": "#ffd08a", "stop-opacity": .55 }), sv("stop", { offset: 1, "stop-color": "#ff5a3c", "stop-opacity": 0 })),
    sv("pattern", { id: "fig-scan", width: 4, height: 3, patternUnits: "userSpaceOnUse" }, sv("rect", { width: 4, height: 1, fill: "rgba(255,220,190,.10)" })),
    sv("filter", { id: "fig-glow", x: "-20%", y: "-20%", width: "140%", height: "140%" },
      sv("feGaussianBlur", { stdDeviation: 2.4, result: "b" }), sv("feMerge", {}, sv("feMergeNode", { in: "b" }), sv("feMergeNode", { in: "SourceGraphic" }))),
    sv("clipPath", { id: "fig-clip" }, sv("path", { d: HALF + " Z" }), sv("path", { d: HALF + " Z", transform: "translate(400 0) scale(-1 1)" })));
  svg.append(defs);
  // Backdrop rings and ticks.
  const back = sv("g", { stroke: "rgba(255,106,77,.22)", fill: "none" });
  for (const [r, dash] of [[110, ""], [150, "2 6"], [190, ""], [230, "1 10"]]) back.append(sv("circle", { cx: 200, cy: 330, r, "stroke-dasharray": dash }));
  back.append(sv("circle", { cx: 200, cy: 330, r: 172, "stroke-dasharray": "40 18 6 18", class: "fig-spin", stroke: "rgba(255,106,77,.35)" }));
  for (let a = 0; a < 360; a += 15) { const t = a * Math.PI / 180; back.append(sv("line", { x1: 200 + 186 * Math.cos(t), y1: 330 + 186 * Math.sin(t), x2: 200 + 194 * Math.cos(t), y2: 330 + 194 * Math.sin(t) })); }
  svg.append(back);
  // Cables from head and shoulders up out of the frame.
  const cab = sv("g", { fill: "none", stroke: "rgba(255,110,90,.45)", "stroke-width": 1.1 });
  for (const [x0, y0, x1, c1] of [[196, 40, 150, 120], [204, 40, 250, 80], [190, 44, 90, 60], [210, 46, 330, 100], [180, 150, 40, 160], [222, 150, 380, 120], [262, 168, 400, 230], [138, 168, 0, 260]]) {
    cab.append(sv("path", { d: `M${x0} ${y0} C${x0 + (x1 - x0) * .2} ${y0 - 60} ${x1} ${c1} ${x1} -10` }));
  }
  svg.append(cab);
  // Platform under the feet.
  const plat = sv("g", { fill: "none" });
  plat.append(sv("ellipse", { cx: 200, cy: 792, rx: 120, ry: 16, stroke: "rgba(255,106,77,.45)" }));
  plat.append(sv("ellipse", { cx: 200, cy: 792, rx: 86, ry: 10, stroke: "rgba(246,207,60,.6)", "stroke-dasharray": "5 7" }));
  plat.append(sv("ellipse", { cx: 200, cy: 742, rx: 70, ry: 9, stroke: "rgba(214,255,90,.5)" }));
  svg.append(plat);
  // Body: fill, core glow, scanlines and inner lines clipped to the outline.
  const body = sv("g", {});
  for (const t of ["", "translate(400 0) scale(-1 1)"]) body.append(sv("path", { d: HALF + " Z", fill: "url(#fig-holo)", transform: t }));
  const inner = sv("g", { "clip-path": "url(#fig-clip)" });
  inner.append(sv("rect", { x: 0, y: 0, width: 400, height: 800, fill: "url(#fig-core)" }));
  inner.append(sv("rect", { x: 0, y: 0, width: 400, height: 800, fill: "url(#fig-scan)" }));
  let s = 7; const rnd = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  const veins = sv("g", { fill: "none", filter: "url(#fig-glow)" });
  for (const mirror of [false, true]) for (const bone of BONES) for (let k = 0; k < 4; k++) {
    const off = (rnd() - .5) * 18; let d = "";
    bone.forEach(([bx, by], i) => {
      const steps = i ? 4 : 1;
      for (let j = 1; j <= steps; j++) {
        const [px, py] = i ? bone[i - 1] : [bx, by];
        const t = j / steps; const x = px + (bx - px) * t + off + (rnd() - .5) * 8, y = py + (by - py) * t + (rnd() - .5) * 6;
        d += `${d ? "L" : "M"}${(mirror ? 400 - x : x).toFixed(1)} ${y.toFixed(1)}`;
      }
    });
    veins.append(sv("path", { d, stroke: k % 2 ? "rgba(255,190,110,.55)" : "rgba(255,90,60,.6)", "stroke-width": k ? .8 : 1.4 }));
  }
  inner.append(veins);
  // Rib arcs and a band of green light across the chest and arms.
  const ribs = sv("g", { fill: "none", stroke: "rgba(255,220,170,.22)" });
  for (let y = 196; y < 300; y += 14) ribs.append(sv("path", { d: `M200 ${y} C228 ${y - 8} 254 ${y + 2} 262 ${y + 14} M200 ${y} C172 ${y - 8} 146 ${y + 2} 138 ${y + 14}` }));
  inner.append(ribs);
  inner.append(sv("path", { d: "M60 270 C120 262 160 286 200 280 C240 274 280 262 340 270", stroke: "rgba(140,255,120,.55)", "stroke-width": 2, fill: "none", filter: "url(#fig-glow)" }));
  body.append(inner);
  for (const t of ["", "translate(400 0) scale(-1 1)"]) {
    const g = sv("g", { transform: t });
    g.append(sv("path", { d: HALF, fill: "none", stroke: "#ff8a66", "stroke-width": 1.3, filter: "url(#fig-glow)" }));
    for (const d of DETAIL) g.append(sv("path", { d, fill: "none", stroke: "rgba(255,170,130,.45)", "stroke-width": .9 }));
    body.append(g);
  }
  body.append(sv("path", { d: "M200 156 V452", stroke: "rgba(255,170,130,.35)", "stroke-width": .9 }));
  svg.append(body);
  for (const sl of BODY_SLOTS) svg.append(sv("circle", { cx: sl.x, cy: sl.y, r: 9, class: "fig-ring", "data-ring": sl.key }));
  return svg;
}


  // ---- slots ------------------------------------------------------------------
  function select(m) { window.netSelectMod(m.id, "canvas"); }
  function tileTitle(m, via) {
    const on = m.status === "installed", up = updatesByMod.get(m.id);
    return [m.name, on ? "enabled" : "disabled", `${m.file_count} file${m.file_count === 1 ? "" : "s"}`,
      `tags: ${(modTags.mods[m.id] || []).join(", ")}`, up ? `${up.latest} available` : null].filter(Boolean).join(" · ")
      + `\nSlot from ${VIA_TITLE[via]}\nClick: select (Netrunner panel shows this mod). Double-click: show in the List view`;
  }
  function thumbBtn(m, via) {
    const b = el("button", { class: `fig-thumb${m.status === "installed" ? "" : " off"}`, "data-mod-id": m.id, title: tileTitle(m, via), onclick: () => select(m) },
      el("span", { class: "fig-img" }, modThumb(m),
        el("span", { class: "fig-badges" }, el("span", { class: `fig-via${via === "tag" ? " tag" : ""}` }, VIA[via]),
          updatesByMod.has(m.id) ? el("span", { class: "canvas-up" }, "▲") : null)),
      el("span", { class: "fig-cap" }, m.name));
    b.addEventListener("dblclick", () => canvasOpenInList(m));
    return b;
  }
  function getMods(sl) {
    showTab("nexus");
    busy(null, async () => {
      await selectSource("nexus", false);
      $("#nexus-search").value = sl.label.split(" ")[0].replace(/s$/, "").toLowerCase() === "full" ? "outfit" : sl.label.split(/ & | /)[0].toLowerCase();
      $("#nexus-search-go").click();
    });
  }
  function slotEl(sl, entries) {
    const ups = entries.filter(([m]) => updatesByMod.has(m.id)).length;
    const n = sl.side === "S" ? Math.min(2, fit) : fit;
    const tiles = entries.slice(0, n).map(([m, via]) => thumbBtn(m, via));
    if (entries.length > n) {
      tiles.push(el("button", { class: "fig-thumb fig-more", "aria-expanded": String(listOpen === sl.key), title: `List all ${entries.length} mods of ${sl.label}`,
        onclick: (e) => { e.stopPropagation(); listOpen = listOpen === sl.key ? null : sl.key; listFilter = ""; renderMods(); } },
      el("span", { class: "fig-img" }, `+${entries.length - n}`), el("span", { class: "fig-cap" }, "List all")));
    } else {
      tiles.push(el("button", { class: "fig-thumb fig-add", title: `Get mods: search Nexus for "${sl.label}"`, onclick: () => getMods(sl) },
        el("span", { class: "fig-img" }, "+"), el("span", { class: "fig-cap" }, "Get mods")));
    }
    return el("div", { class: `fig-slot${entries.length ? "" : " empty"}${listOpen === sl.key ? " open" : ""}`, "data-slot": sl.key,
      onmouseover: (e) => { const t = e.target.closest("[data-mod-id]"); hot(t ? { mod: Number(t.dataset.modId) } : { slot: sl.key }); },
      onmouseleave: () => hot({}) },
    el("div", { class: "fig-lab" },
      el("span", { class: "fig-name" }, sl.label),
      el("span", { class: "fig-sub" }, entries.length ? ["Mods", el("span", { class: "fig-n" }, entries.length)] : "No mods",
        ups ? el("span", { class: "canvas-up", title: `${ups} with an update available` }, `▲${ups}`) : null)),
    el("div", { class: "fig-thumbs" }, tiles),
    listOpen === sl.key ? slotList(sl, entries) : null);
  }

  // Every mod of a slot in a list that scrolls inside the figure.
  function slotList(sl, entries) {
    const rows = el("div", { class: "fig-rows" });
    const fill = () => {
      const q = listFilter.toLowerCase();
      const shown = entries.filter(([m]) => m.name.toLowerCase().includes(q));
      rows.replaceChildren(...shown.map(([m, via]) => {
        const r = el("button", { class: `fig-row${m.status === "installed" ? "" : " off"}`, "data-mod-id": m.id, title: tileTitle(m, via), onclick: () => select(m) },
          modThumb(m), el("span", { class: "fig-rname" }, m.name), el("span", { class: `fig-via${via === "tag" ? " tag" : ""}` }, VIA[via]),
          updatesByMod.has(m.id) ? el("span", { class: "canvas-up" }, "▲") : null, el("span", { class: "canvas-files" }, m.file_count));
        r.addEventListener("dblclick", () => canvasOpenInList(m));
        return r;
      }));
      if (!shown.length) rows.append(el("div", { class: "muted small fig-none" }, "No match"));
      window.netDecorate?.();
    };
    fill();
    const tags = [...new Set(entries.flatMap(([m]) => (modTags.mods[m.id] || []).filter((t) => TAG_SLOTS[t]?.includes(sl.key))))];
    return el("div", { class: "fig-list", role: "dialog", "aria-label": `${sl.label}: ${entries.length} mods`, onclick: (e) => e.stopPropagation() },
      el("div", { class: "fig-lhead" }, el("span", { class: "fig-lt" }, `${sl.label} · ${entries.length}`),
        entries.length > 8 ? el("input", { type: "search", class: "inline", placeholder: "Filter by name", "aria-label": `Filter ${sl.label}`, value: listFilter,
          oninput: (e) => { listFilter = e.target.value; fill(); } }) : null,
        tags.length === 1 ? el("button", { title: `Show the mods tagged ${tags[0]} in the List view`, onclick: () => {
          $("#mods-filter").value = ""; $("#mods-category").value = `tag:${tags[0]}`; setModsView("list"); renderMods();
        } }, "Open in List") : null,
        el("button", { title: "Close", "aria-label": "Close", onclick: () => { listOpen = null; renderMods(); } }, "✕")),
      rows);
  }

  // ---- render -----------------------------------------------------------------
  window.renderModsFigure = (list) => {
    loadHints().catch(() => {});
    const bySlot = new Map(FIG_SLOTS.map((s) => [s.key, []]));
    let placed = 0;
    for (const m of list) {
      const s = slotsOf(m);
      if (s.size) placed++;
      for (const [k, via] of s) bySlot.get(k)?.push([m, via]);
    }
    const withArchives = hints?.with_archives || 0, read = hints?.archives_read || 0;
    const head = el("div", { class: "fig-head" },
      el("button", { title: "Back to the tag columns", onclick: close }, "‹ Board"),
      el("span", { class: "fig-title" }, "Character"),
      el("span", { class: "muted small" }, "Slot from: ", el("span", { class: "fig-via" }, "FILES"), " TweakXL item type or depot paths · ",
        el("span", { class: "fig-via" }, "NAME"), " words in the mod name · ", el("span", { class: "fig-via tag" }, "TAG"), " tag only"),
      withArchives ? el("span", { class: "fig-wk" },
        el("button", {
          disabled: wkCli ? null : "", title: wkCli
            ? "List the .archive files of the mods on the figure with the WolvenKit CLI; their depot paths then set the slots (cached per file)"
            : "Needs the WolvenKit CLI and .NET: Netrunner panel › WolvenKit",
          onclick: (e) => busy(e.target, async () => {
            const ids = list.filter((m) => slotsOf(m).size).map((m) => m.id);
            const r = await invoke("figure_read_archives", { modIds: ids });
            toast(`${r.tool}: ${r.read} archive${r.read === 1 ? "" : "s"} read${r.failed.length ? `, ${r.failed.length} failed` : ""}`);
            hintsKey = ""; await loadHints(); renderMods();
          }),
        }, "Read archive paths"),
        el("span", { class: "muted small" }, `${read} of ${withArchives} mods with archives read by WolvenKit`)) : null);
    const side = (s) => el("div", { class: `fig-side ${s}` }, BODY_SLOTS.filter((x) => x.side === s).map((x) => slotEl(x, bySlot.get(x.key))));
    const fig = el("div", { class: "fig-body" }, side("L"), el("div", { class: "fig-draw" }, figureSvg()), side("R"), sv("svg", { class: "fig-wires" }));
    const strip = el("div", { class: "fig-strip" }, el("div", { class: "fig-strip-h" }, "Not tied to a body part"),
      FIG_SLOTS.filter((x) => x.side === "S").map((x) => slotEl(x, bySlot.get(x.key))));
    const off = list.length - placed;
    $("#mods-canvas").replaceChildren(el("div", { class: "fig" }, head, fig, strip,
      off ? el("p", { class: "muted small" }, `${off} mod${off === 1 ? "" : "s"} without a Characters or Clothing tag: on the board.`) : null));
    $("#canvas-summary").textContent = `${list.length} mod${list.length === 1 ? "" : "s"} · ${placed} on the figure`;
    requestAnimationFrame(() => {
      const r = fig.querySelector(".fig-side.R")?.getBoundingClientRect().width || 0;
      // Beside the label when two or more fit there, else on a line under it.
      const inline = Math.floor((r - 150) / 76) - 1;
      const want = Math.max(1, Math.min(MAX_THUMBS, inline >= 2 ? inline : Math.floor((r + 6) / 76) - 1));
      if (want !== fit) { fit = want; return renderMods(); }
      const o = fig.parentNode.querySelector(".fig-slot.open");
      if (o) {
        o.classList.toggle("up", o.getBoundingClientRect().top > window.innerHeight * 0.5);
        o.querySelector(".fig-list input")?.focus({ preventScroll: true });
      }
      drawWires();
      window.netDecorate?.();
    });
  };

  function drawWires() {
    const fig = document.querySelector("#mods-canvas .fig-body"), wires = fig?.querySelector(".fig-wires"), svg = fig?.querySelector(".fig-draw svg");
    if (!wires || !svg || getComputedStyle(wires).display === "none") return;
    wires.replaceChildren();
    const fr = fig.getBoundingClientRect(), m = svg.getScreenCTM();
    if (!m) return;
    for (const sl of BODY_SLOTS) {
      const row = fig.querySelector(`.fig-slot[data-slot="${sl.key}"] .fig-thumbs`);
      if (!row) continue;
      const br = row.getBoundingClientRect();
      const p = new DOMPoint(sl.x, sl.y).matrixTransform(m);
      const ax = p.x - fr.left, ay = p.y - fr.top;
      const sx = (sl.side === "L" ? br.right + 4 : br.left - 4) - fr.left, sy = br.top - fr.top + 35;
      const mx = sl.side === "L" ? sx + 14 : sx - 14;
      wires.append(sv("path", { d: `M${sx} ${sy} H${mx} L${ax} ${ay}`, "data-wire": sl.key }));
      wires.append(sv("circle", { cx: ax, cy: ay, r: 3, "data-wire": sl.key }));
      wires.append(sv("circle", { cx: sx, cy: sy, r: 2, "data-wire": sl.key }));
    }
    hot(cur);
  }

  function hot(state) {
    cur = state;
    const { slot, mod } = state;
    let keys;
    if (slot) keys = new Set([slot]);
    else if (mod != null) { const m = mods.find((x) => x.id === mod); keys = new Set(m ? slotsOf(m).keys() : []); }
    else keys = new Set(lit ? [lit] : []);
    const root = $("#mods-canvas");
    root.querySelectorAll("[data-slot]").forEach((e) => e.classList.toggle("hot", keys.has(e.dataset.slot)));
    root.querySelectorAll("[data-wire]").forEach((e) => e.classList.toggle("hot", keys.has(e.dataset.wire)));
    root.querySelectorAll("[data-ring]").forEach((e) => e.classList.toggle("hot", keys.has(e.dataset.ring)));
    root.querySelectorAll(".fig-thumb[data-mod-id], .fig-row").forEach((e) => e.classList.toggle("hot", mod != null && Number(e.dataset.modId) === mod));
  }

  let resizeT;
  new ResizeObserver(() => {
    if (!open || modsView !== "canvas") return;
    drawWires();
    clearTimeout(resizeT);
    resizeT = setTimeout(renderMods, 200);
  }).observe($("#mods-canvas"));
  document.addEventListener("click", (e) => { if (listOpen && !e.target.closest(".fig-list")) { listOpen = null; renderMods(); } });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && listOpen) { listOpen = null; renderMods(); } });
})();
