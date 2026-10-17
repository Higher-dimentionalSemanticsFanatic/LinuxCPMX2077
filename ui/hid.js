// HID input page in Installed mods: CPMX2077 HID Input, a RED4ext plugin
// carried in this app, adds game controllers the game does not read
// (pedals, wheels, flight sticks, button boxes) to its XInput gamepad.
// The page lists the controllers connected here with a drawing of each and
// their live inputs, binds inputs to gamepad targets (pick a target on the
// gamepad drawing, then move or press the input), and saves the bindings
// file the plugin reads. The plugin is a tracked mod, so Disable and
// Uninstall reuse the mod table's actions. Loaded after app.js and built
// from its helpers.

const SVGNS = "http://www.w3.org/2000/svg";

// Targets, with the side of the gamepad drawing their callout sits on.
const HID_TARGETS = [
  { id: "left_trigger", label: "Left trigger", hint: "vehicle: brake, reverse", side: "l", kind: "trigger" },
  { id: "left_shoulder", label: "Left bumper", side: "l", kind: "button" },
  { id: "left_x", label: "Left stick X", hint: "vehicle: steer", side: "l", kind: "stick" },
  { id: "left_y", label: "Left stick Y", side: "l", kind: "stick" },
  { id: "left_thumb", label: "Left stick click", side: "l", kind: "button" },
  { id: "dpad", label: "D-pad (hat switch)", side: "l", kind: "hat" },
  { id: "dpad_up", label: "D-pad up", side: "l", kind: "button" },
  { id: "dpad_down", label: "D-pad down", side: "l", kind: "button" },
  { id: "dpad_left", label: "D-pad left", side: "l", kind: "button" },
  { id: "dpad_right", label: "D-pad right", side: "l", kind: "button" },
  { id: "back", label: "View / Back", side: "l", kind: "button" },
  { id: "right_trigger", label: "Right trigger", hint: "vehicle: accelerate", side: "r", kind: "trigger" },
  { id: "right_shoulder", label: "Right bumper", side: "r", kind: "button" },
  { id: "y", label: "Y", side: "r", kind: "button" },
  { id: "x", label: "X", side: "r", kind: "button" },
  { id: "b", label: "B", side: "r", kind: "button" },
  { id: "a", label: "A", side: "r", kind: "button" },
  { id: "right_x", label: "Right stick X", side: "r", kind: "stick" },
  { id: "right_y", label: "Right stick Y", side: "r", kind: "stick" },
  { id: "right_thumb", label: "Right stick click", side: "r", kind: "button" },
  { id: "start", label: "Menu / Start", side: "r", kind: "button" },
];
const HID_TARGET = Object.fromEntries(HID_TARGETS.map((t) => [t.id, t]));
const HID_BITS = { dpad_up: 0x1, dpad_down: 0x2, dpad_left: 0x4, dpad_right: 0x8, start: 0x10, back: 0x20,
  left_thumb: 0x40, right_thumb: 0x80, left_shoulder: 0x100, right_shoulder: 0x200, a: 0x1000, b: 0x2000, x: 0x4000, y: 0x8000 };
const HID_KINDS = { pedals: "Pedals", wheel: "Wheel", stick: "Flight stick", throttle: "Throttle", buttonbox: "Button box", gamepad: "Gamepad", generic: "Other device" };
const HID_CURVES = [[1, "Linear"], [1.6, "Progressive"], [2.5, "Strong"]];
const HID_HALVES = [["full", "Whole axis"], ["positive", "Rest to + (right, up)"], ["negative", "Rest to − (left, down)"]];

let hidState = null;
let hidDevices = [];
let hidLive = new Map();
let hidBind = null; // the bindings file being edited
let hidDirty = false;
let hidListen = null; // { target, base: Map(id → live) }
let hidPolling = false;
let hidKinds = loadPref("hidKinds", {});
const hidPictures = new Map(); // device id → update(live)
let hidPadUpdate = null;
const hidRows = []; // [{ binding, bar }] for the bindings table's live output

function sv(tag, attrs = {}, ...children) {
  const e = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v !== null && v !== undefined) e.setAttribute(k, v);
  for (const c of children.flat()) {
    if (c === null || c === undefined) continue;
    e.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return e;
}

function hidVisible() {
  return modsSub === "hid" && $("#tab-mods").classList.contains("active") && !$("#hid").classList.contains("hidden");
}

async function loadHid() {
  const box = $("#hid");
  if (!currentGame) return box.classList.add("hidden");
  const gameId = currentGame.id;
  const [s, b] = await Promise.all([invoke("hid_state", { gameId }), invoke("hid_bindings", { gameId })]);
  if (currentGame?.id !== gameId) return;
  hidState = s;
  if (!hidDirty || hidBind?.gameId !== gameId) {
    hidBind = { ...b, gameId };
    hidDirty = false;
  }
  box.classList.remove("hidden");
  if (!hidDevices.length || hidVisible()) hidDevices = await invoke("hid_devices");
  renderHid();
  hidStartPolling();
}

function hidStartPolling() {
  if (hidPolling || !hidVisible()) return;
  hidPolling = true;
  (async () => {
    while (hidVisible()) {
      try {
        const live = await invoke("hid_poll");
        hidLive = new Map(live.map((l) => [l.id, l]));
        hidTick();
      } catch { /* the next poll retries */ }
      await new Promise((r) => setTimeout(r, 60));
    }
    hidPolling = false;
    hidListen = null;
  })();
}

// ---- what the bindings add, as the plugin works it out ----------------------

function hidFromRest(v, b) {
  v = Math.min(1, Math.max(0, b.invert ? 1 - v : v));
  const dz = Math.min(0.5, Math.max(0, b.deadzone || 0));
  v = v <= dz ? 0 : (v - dz) / (1 - dz);
  return Math.pow(v, b.curve || 1);
}

function hidCentred(v, b) {
  const s = (Math.min(1, Math.max(0, v)) * 2 - 1) * (b.invert ? -1 : 1);
  const dz = Math.min(0.5, Math.max(0, b.deadzone || 0));
  const m = Math.abs(s) <= dz ? 0 : (Math.abs(s) - dz) / (1 - dz);
  return Math.pow(m, b.curve || 1) * Math.sign(s);
}

const HID_HAT_BITS = [0x1, 0x1 | 0x8, 0x8, 0x2 | 0x8, 0x2, 0x2 | 0x4, 0x4, 0x1 | 0x4];

// One binding's output now: a number (trigger 0..1, stick −1..1) or button bits.
function hidOutput(b) {
  const live = hidLive.get(b.device);
  if (!live) return null;
  const i = b.input.index;
  if (b.input.kind === "hat") return { bits: b.target === "dpad" ? (HID_HAT_BITS[live.hats[i]] || 0) : 0 };
  const raw = b.input.kind === "axis" ? live.axes[i] : live.buttons[i] ? 1 : 0;
  if (raw === undefined) return null;
  const t = HID_TARGET[b.target];
  if (t.kind === "button") return { bits: hidFromRest(raw, b) > 0.5 ? HID_BITS[b.target] : 0 };
  if (t.kind === "stick") {
    const half = b.half || "full";
    const v = half === "full" && b.input.kind === "axis" ? hidCentred(raw, b) : half === "negative" ? -hidFromRest(raw, b) : hidFromRest(raw, b);
    return { value: v };
  }
  return { value: hidFromRest(raw, b) };
}

function hidContribution() {
  const c = { buttons: 0, left_trigger: 0, right_trigger: 0, left_x: 0, left_y: 0, right_x: 0, right_y: 0 };
  if (!hidBind?.enabled) return c;
  for (const b of hidBind.bindings) {
    const o = hidOutput(b);
    if (!o) continue;
    if (o.bits !== undefined) c.buttons |= o.bits;
    else if (b.target.endsWith("trigger")) c[b.target] = Math.max(c[b.target], o.value);
    else c[b.target] = Math.min(1, Math.max(-1, c[b.target] + o.value));
  }
  return c;
}

// ---- listening for an input ----------------------------------------------------

function hidStartListen(target) {
  hidListen = { target, base: new Map([...hidLive].map(([k, v]) => [k, structuredClone(v)])) };
  renderHid();
}

function hidStopListen() {
  hidListen = null;
  renderHid();
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && hidListen) hidStopListen();
});

// The first input that moved since listening began: an axis by a quarter of
// its range, a button pressed, a hat pushed.
function hidDetect() {
  const t = HID_TARGET[hidListen.target];
  for (const [id, live] of hidLive) {
    const base = hidListen.base.get(id);
    if (!base) continue;
    if (t.kind === "hat") {
      const h = live.hats.findIndex((x, i) => x !== null && base.hats[i] === null);
      if (h >= 0) return { id, input: { kind: "hat", index: h } };
      continue;
    }
    for (let i = 0; i < live.axes.length; i++) {
      if (Math.abs(live.axes[i] - base.axes[i]) > 0.25) return { id, input: { kind: "axis", index: i }, rest: base.axes[i] };
    }
    const b = live.buttons.findIndex((x, i) => x && !base.buttons[i]);
    if (b >= 0) return { id, input: { kind: "button", index: b } };
    if (t.id.startsWith("dpad_")) {
      // A hat bound to one direction binds the whole d-pad.
      const h = live.hats.findIndex((x, i) => x !== null && base.hats[i] === null);
      if (h >= 0) return { id, input: { kind: "hat", index: h }, target: "dpad" };
    }
  }
  return null;
}

function hidDeviceEntry(id) {
  const d = hidDevices.find((x) => x.id === id);
  if (!hidBind.devices.some((x) => x.id === id) && d) {
    hidBind.devices.push({ id, vid: d.vid, pid: d.pid, instance: d.instance, name: d.name, hid_axes: d.hid_axes, kind: hidKindOf(d) });
  }
  return d;
}

function hidAddBinding(found) {
  const d = hidDeviceEntry(found.id);
  const target = found.target || hidListen.target;
  const t = HID_TARGET[target];
  const input = { ...found.input };
  if (input.kind === "axis") {
    const a = d?.axes[input.index];
    if (a?.usage !== null && a?.usage !== undefined) { input.usage = a.usage; input.occurrence = a.occurrence; }
  }
  const b = { device: found.id, input, target, invert: false, deadzone: 0.03, curve: 1, half: "full" };
  const rest = found.rest;
  if (input.kind === "axis") {
    const centred = rest > 0.3 && rest < 0.7;
    if (t.kind === "stick" && centred) b.deadzone = 0.05;
    else if (rest > 0.5) b.invert = true; // pedal resting at the top of its range
    if (t.kind === "stick" && !centred) b.half = "positive";
  } else if (t.kind === "stick") {
    b.half = "positive";
  }
  hidBind.bindings.push(b);
  hidDirty = true;
}

// ---- drawings ---------------------------------------------------------------------

function hidKindOf(d) {
  return hidKinds[d.id] || hidBind?.devices.find((x) => x.id === d.id)?.kind || d.kind;
}

// The axis a drawing animates: the first whose label matches, else `fallback`.
function hidAxisIndex(d, labels, fallback) {
  const i = d.axes.findIndex((a) => labels.includes(a.label));
  return i >= 0 ? i : fallback;
}

function drawPedals(d) {
  const n = Math.min(4, Math.max(2, d.axes.length));
  const w = 240 / n;
  const parts = [];
  const g = [];
  for (let i = 0; i < n; i++) {
    const cx = w * i + w / 2;
    const fill = sv("rect", { class: "hid-live", x: cx - 15, y: 112, width: 30, height: 0, rx: 3 });
    const pad = sv("g", {},
      sv("line", { class: "hid-stroke", x1: cx, y1: 118, x2: cx, y2: 70 }),
      sv("rect", { class: "hid-part", x: cx - 18, y: 26, width: 36, height: 50, rx: 7 }),
      sv("line", { class: "hid-grip", x1: cx - 10, y1: 38, x2: cx + 10, y2: 38 }),
      sv("line", { class: "hid-grip", x1: cx - 10, y1: 48, x2: cx + 10, y2: 48 }),
      sv("line", { class: "hid-grip", x1: cx - 10, y1: 58, x2: cx + 10, y2: 58 }));
    g.push(sv("rect", { class: "hid-base", x: cx - w / 2 + 6, y: 118, width: w - 12, height: 14, rx: 3 }), pad,
      sv("rect", { class: "hid-gauge", x: cx - 15, y: 82, width: 30, height: 30, rx: 3 }), fill,
      sv("text", { class: "hid-label", x: cx, y: 146, "text-anchor": "middle" }, d.axes[i]?.label || "—"));
    parts.push({ pad, fill, axis: i });
  }
  return { svg: sv("svg", { viewBox: "0 0 240 150", class: "hid-pic" }, g), update(l) {
    for (const p of parts) {
      const v = l?.axes[p.axis] ?? 0;
      const b = hidBind?.bindings.find((x) => x.device === d.id && x.input.kind === "axis" && x.input.index === p.axis);
      const press = b ? hidFromRest(v, { ...b, curve: 1, deadzone: 0 }) : v;
      p.pad.setAttribute("transform", `translate(0 ${(press * 14).toFixed(1)})`);
      p.fill.setAttribute("y", (112 - press * 30).toFixed(1));
      p.fill.setAttribute("height", (press * 30).toFixed(1));
    }
  } };
}

function drawWheel(d) {
  const axis = hidAxisIndex(d, ["Steering", "X axis"], 0);
  const rim = sv("g", {},
    sv("circle", { class: "hid-rim", cx: 120, cy: 74, r: 56 }),
    sv("path", { class: "hid-part", d: "M66 82 L104 78 L104 96 L82 112 Z" }),
    sv("path", { class: "hid-part", d: "M174 82 L136 78 L136 96 L158 112 Z" }),
    sv("rect", { class: "hid-part", x: 112, y: 92, width: 16, height: 30, rx: 4 }),
    sv("circle", { class: "hid-part", cx: 120, cy: 84, r: 18 }),
    sv("rect", { class: "hid-live", x: 116, y: 14, width: 8, height: 10, rx: 2 }));
  return { svg: sv("svg", { viewBox: "0 0 240 150", class: "hid-pic" }, rim,
    sv("text", { class: "hid-label", x: 120, y: 146, "text-anchor": "middle" }, d.axes[axis]?.label || "")), update(l) {
    const v = l?.axes[axis] ?? 0.5;
    rim.setAttribute("transform", `rotate(${((v - 0.5) * 270).toFixed(1)} 120 74)`);
  } };
}

function drawStick(d) {
  const ax = hidAxisIndex(d, ["X axis"], 0);
  const ay = hidAxisIndex(d, ["Y axis"], 1);
  const shaft = sv("line", { class: "hid-stroke thick", x1: 120, y1: 118, x2: 120, y2: 54 });
  const grip = sv("g", {},
    sv("path", { class: "hid-part", d: "M104 60 Q100 30 112 18 L130 16 Q142 26 138 60 Z" }),
    sv("circle", { class: "hid-part", cx: 121, cy: 28, r: 7 }));
  const hatDot = sv("circle", { class: "hid-live", cx: 121, cy: 28, r: 0 });
  grip.append(hatDot);
  return { svg: sv("svg", { viewBox: "0 0 240 150", class: "hid-pic" },
    sv("ellipse", { class: "hid-base", cx: 120, cy: 126, rx: 70, ry: 14 }),
    sv("ellipse", { class: "hid-part", cx: 120, cy: 118, rx: 24, ry: 7 }), shaft, grip,
    sv("text", { class: "hid-label", x: 120, y: 148, "text-anchor": "middle" }, [[d.axes.length, "axis", "axes"], [d.buttons, "button", "buttons"], [d.hats, "hat", "hats"]].map(([n, one, many]) => `${n} ${n === 1 ? one : many}`).join(" · "))), update(l) {
    const dx = ((l?.axes[ax] ?? 0.5) - 0.5) * 50;
    const dy = ((l?.axes[ay] ?? 0.5) - 0.5) * 24;
    shaft.setAttribute("x2", (120 + dx).toFixed(1));
    shaft.setAttribute("y2", (54 + dy).toFixed(1));
    grip.setAttribute("transform", `translate(${dx.toFixed(1)} ${dy.toFixed(1)})`);
    const h = l?.hats[0];
    if (h === null || h === undefined) hatDot.setAttribute("r", 0);
    else {
      const a = (h * 45 - 90) * Math.PI / 180;
      hatDot.setAttribute("r", 3);
      hatDot.setAttribute("cx", (121 + Math.cos(a) * 5).toFixed(1));
      hatDot.setAttribute("cy", (28 + Math.sin(a) * 5).toFixed(1));
    }
  } };
}

function drawThrottle(d) {
  const axis = hidAxisIndex(d, ["Throttle", "Z axis", "Slider"], 0);
  const lever = sv("g", {},
    sv("line", { class: "hid-stroke thick", x1: 0, y1: 100, x2: 14, y2: 44 }),
    sv("rect", { class: "hid-part", x: -6, y: 18, width: 40, height: 30, rx: 8 }));
  return { svg: sv("svg", { viewBox: "0 0 240 150", class: "hid-pic" },
    sv("rect", { class: "hid-base", x: 30, y: 96, width: 180, height: 34, rx: 6 }),
    sv("line", { class: "hid-slot", x1: 56, y1: 100, x2: 184, y2: 100 }), lever,
    sv("text", { class: "hid-label", x: 120, y: 146, "text-anchor": "middle" }, d.axes[axis]?.label || "")), update(l) {
    const v = l?.axes[axis] ?? 0;
    lever.setAttribute("transform", `translate(${(56 + v * 128).toFixed(1)} 0)`);
  } };
}

function drawButtonBox(d) {
  const n = Math.min(32, Math.max(1, d.buttons));
  const cols = Math.min(8, n);
  const rows = Math.ceil(n / cols);
  const lamps = [];
  for (let i = 0; i < n; i++) {
    const cx = 120 - (cols - 1) * 12 + (i % cols) * 24;
    const cy = 75 - (rows - 1) * 12 + Math.floor(i / cols) * 24;
    lamps.push(sv("circle", { class: "hid-lamp", cx, cy, r: 8 }));
  }
  return { svg: sv("svg", { viewBox: "0 0 240 150", class: "hid-pic" },
    sv("rect", { class: "hid-base", x: 10, y: 10, width: 220, height: 130, rx: 10 }), lamps), update(l) {
    lamps.forEach((c, i) => c.classList.toggle("on", !!l?.buttons[i]));
  } };
}

function drawGeneric(d) {
  const n = Math.min(8, d.axes.length);
  const bars = [];
  const g = [sv("rect", { class: "hid-base", x: 10, y: 10, width: 220, height: 130, rx: 10 })];
  for (let i = 0; i < n; i++) {
    const x = 120 - (n - 1) * 13 + i * 26;
    const bar = sv("rect", { class: "hid-live", x: x - 7, y: 112, width: 14, height: 0, rx: 2 });
    g.push(sv("rect", { class: "hid-gauge", x: x - 7, y: 32, width: 14, height: 80, rx: 2 }), bar);
    bars.push(bar);
  }
  if (!n) g.push(sv("text", { class: "hid-label", x: 120, y: 80, "text-anchor": "middle" }, `${d.buttons} buttons`));
  return { svg: sv("svg", { viewBox: "0 0 240 150", class: "hid-pic" }, g), update(l) {
    bars.forEach((b, i) => {
      const v = l?.axes[i] ?? 0;
      b.setAttribute("y", (112 - v * 80).toFixed(1));
      b.setAttribute("height", (v * 80).toFixed(1));
    });
  } };
}

// A generic gamepad. With `callouts`, the binding view: every target labelled
// at the side with what is bound to it, clickable to listen for an input.
function drawGamepad(callouts) {
  const ox = callouts ? 290 : 0;
  const oy = callouts ? 40 : -15;
  const p = (x, y) => [x + ox, y + oy];
  const parts = {};
  const g = [];
  const [bx, by] = p(0, 0);
  g.push(sv("path", { class: "hid-base", d: `M${bx + 70} ${by + 80} Q${bx + 120} ${by + 62} ${bx + 170} ${by + 80} Q${bx + 225} ${by + 72} ${bx + 236} ${by + 120}
    L${bx + 232} ${by + 190} Q${bx + 226} ${by + 218} ${bx + 200} ${by + 206} L${bx + 176} ${by + 170} L${bx + 64} ${by + 170} L${bx + 40} ${by + 206}
    Q${bx + 14} ${by + 218} ${bx + 8} ${by + 190} L${bx + 4} ${by + 120} Q${bx + 15} ${by + 72} ${bx + 70} ${by + 80} Z` }));
  const trig = (id, x) => {
    const [tx, ty] = p(x, 46);
    const fill = sv("rect", { class: "hid-live", x: tx, y: ty + 14, width: 34, height: 0, rx: 3 });
    g.push(sv("rect", { class: "hid-part", x: tx, y: ty, width: 34, height: 14, rx: 4 }), fill);
    parts[id] = { anchor: [tx + 17, ty + 2], update: (v) => { fill.setAttribute("y", (ty + 14 - v * 14).toFixed(1)); fill.setAttribute("height", (v * 14).toFixed(1)); } };
  };
  trig("left_trigger", 40);
  trig("right_trigger", 166);
  const bumper = (id, x) => {
    const [tx, ty] = p(x, 64);
    const r = sv("rect", { class: "hid-lamp", x: tx, y: ty, width: 46, height: 10, rx: 4 });
    g.push(r);
    parts[id] = { anchor: [tx + 23, ty + 5], lamp: r };
  };
  bumper("left_shoulder", 34);
  bumper("right_shoulder", 160);
  const stick = (prefix, x, y) => {
    const [cx, cy] = p(x, y);
    const knob = sv("circle", { class: "hid-lamp", cx, cy, r: 11 });
    g.push(sv("circle", { class: "hid-part", cx, cy, r: 20 }), knob);
    parts[`${prefix}_x`] = { anchor: [cx + 20, cy] };
    parts[`${prefix}_y`] = { anchor: [cx, cy - 20] };
    parts[`${prefix}_thumb`] = { anchor: [cx, cy + 11], lamp: knob };
    parts[`${prefix}_stick`] = { knob, cx, cy };
  };
  stick("left", 66, 112);
  stick("right", 150, 150);
  const [dx, dy] = p(88, 150);
  const dpad = { dpad_up: [dx - 6, dy - 22, 12, 16], dpad_down: [dx - 6, dy + 6, 12, 16], dpad_left: [dx - 22, dy - 6, 16, 12], dpad_right: [dx + 6, dy - 6, 16, 12] };
  g.push(sv("rect", { class: "hid-part", x: dx - 6, y: dy - 6, width: 12, height: 12 }));
  for (const [id, [x, y, w, h]] of Object.entries(dpad)) {
    const r = sv("rect", { class: "hid-lamp", x, y, width: w, height: h, rx: 2 });
    g.push(r);
    parts[id] = { anchor: [x + w / 2, y + h / 2], lamp: r };
  }
  parts.dpad = { anchor: [dx - 22, dy] };
  const face = { y: [174, 96], x: [158, 112], b: [190, 112], a: [174, 128] };
  for (const [id, [x, y]] of Object.entries(face)) {
    const [cx, cy] = p(x, y);
    const c = sv("circle", { class: "hid-lamp", cx, cy, r: 8 });
    g.push(c, sv("text", { class: "hid-face", x: cx, y: cy + 3.5, "text-anchor": "middle" }, id.toUpperCase()));
    parts[id] = { anchor: [cx, cy], lamp: c };
  }
  for (const [id, x] of [["back", 100], ["start", 140]]) {
    const [cx, cy] = p(x, 104);
    const c = sv("rect", { class: "hid-lamp", x: cx - 7, y: cy - 4, width: 14, height: 8, rx: 4 });
    g.push(c);
    parts[id] = { anchor: [cx, cy], lamp: c };
  }

  const labels = [];
  if (callouts) {
    for (const side of ["l", "r"]) {
      const list = HID_TARGETS.filter((t) => t.side === side);
      list.forEach((t, i) => {
        const y = 22 + i * 30;
        const x = side === "l" ? 6 : 594;
        const anchor = parts[t.id].anchor;
        const edge = side === "l" ? x + 214 : x;
        const bound = hidBind.bindings.filter((b) => b.target === t.id).map(hidInputName)
          .concat((hidState?.menu?.bindings || []).filter((b) => b.target === t.id).map((b) => `Controller ${b.controller} (game menu)`));
        const boundText = bound.length ? `← ${bound.map((n) => hidShort(n, bound.length > 1 ? 18 : 38)).join(", ")}` : "not bound";
        const listening = hidListen?.target === t.id;
        const grp = sv("g", { class: `hid-callout${listening ? " listening" : ""}${bound.length ? " bound" : ""}`, tabindex: 0, role: "button",
          "aria-label": `${t.label}: ${bound.join(", ") || "not bound"}. Bind an input` },
          sv("title", {}, listening ? "Move or press the input to bind; Esc cancels"
            : `${bound.length ? `Bound: ${bound.join(", ")}\n` : ""}Bind an input to ${t.label}: click, then move or press it`),
          sv("polyline", { class: "hid-leader", points: `${edge},${y + 6} ${side === "l" ? edge + 12 : edge - 12},${y + 6} ${anchor[0]},${anchor[1]}` }),
          sv("rect", { class: "hid-callout-box", x, y: y - 10, width: 220, height: 27, rx: 3 }),
          sv("text", { class: "hid-callout-title", x: x + 8, y: y + 2 }, listening ? `${t.label}: move or press…` : t.label,
            t.hint && !listening ? sv("tspan", { class: "hid-hint" }, `  ${t.hint}`) : null),
          sv("text", { class: "hid-callout-bound", x: x + 8, y: y + 13 }, hidShort(boundText, 44)));
        grp.addEventListener("click", () => (listening ? hidStopListen() : hidStartListen(t.id)));
        grp.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); grp.dispatchEvent(new MouseEvent("click")); } });
        labels.push(grp);
      });
    }
  }
  const svg = callouts
    ? sv("svg", { viewBox: "0 0 820 352", class: "hid-pad" }, labels, g)
    : sv("svg", { viewBox: "0 0 240 150", class: "hid-pic" }, g);
  const update = (c) => {
    parts.left_trigger.update(c.left_trigger);
    parts.right_trigger.update(c.right_trigger);
    for (const s of ["left", "right"]) {
      const k = parts[`${s}_stick`];
      k.knob.setAttribute("cx", (k.cx + c[`${s}_x`] * 9).toFixed(1));
      k.knob.setAttribute("cy", (k.cy - c[`${s}_y`] * 9).toFixed(1));
    }
    for (const [id, bit] of Object.entries(HID_BITS)) parts[id].lamp?.classList.toggle("on", (c.buttons & bit) !== 0);
  };
  return { svg, update };
}

// A gamepad device's own values, shaped like a contribution for drawGamepad.
function hidPadValues(l) {
  const a = (i) => l?.axes[i] ?? 0.5;
  const c = { buttons: 0, left_trigger: l?.axes[2] ?? 0, right_trigger: l?.axes[5] ?? 0,
    left_x: a(0) * 2 - 1, left_y: 1 - a(1) * 2, right_x: a(3) * 2 - 1, right_y: 1 - a(4) * 2 };
  const order = ["a", "b", "x", "y", "left_shoulder", "right_shoulder", "back", "start", "left_thumb", "right_thumb"];
  order.forEach((id, i) => { if (l?.buttons[i]) c.buttons |= HID_BITS[id]; });
  if (l?.hats[0] !== null && l?.hats[0] !== undefined) c.buttons |= HID_HAT_BITS[l.hats[0]];
  return c;
}

function drawDevice(d) {
  const kind = hidKindOf(d);
  if (kind === "gamepad") {
    const p = drawGamepad(false);
    return { svg: p.svg, update: (l) => p.update(hidPadValues(l)) };
  }
  return ({ pedals: drawPedals, wheel: drawWheel, stick: drawStick, throttle: drawThrottle, buttonbox: drawButtonBox }[kind] || drawGeneric)(d);
}

// ---- page -------------------------------------------------------------------------

function hidInputName(b) {
  const d = hidDevices.find((x) => x.id === b.device) || hidBind.devices.find((x) => x.id === b.device);
  const name = d?.name || b.device;
  const i = b.input.index;
  const what = b.input.kind === "axis" ? (d?.axes?.[i]?.label || `Axis ${i + 1}`)
    : b.input.kind === "button" ? `Button ${i + 1}` : `Hat ${i + 1}`;
  return `${name}: ${what}`;
}

// HID axis usages (page << 16 | id) as the plugin reports them.
const HID_USAGES = { 0x10030: "X axis", 0x10031: "Y axis", 0x10032: "Z axis", 0x10033: "X rotation", 0x10034: "Y rotation",
  0x10035: "Z rotation", 0x10036: "Slider", 0x10037: "Dial", 0x10038: "Wheel", 0x200ba: "Rudder", 0x200bb: "Throttle",
  0x200c4: "Accelerator", 0x200c5: "Brake", 0x200c6: "Clutch", 0x200c8: "Steering" };

// The controller the plugin numbered `n` for the in-game menu in its last run.
function hidMenuController(n) {
  return hidState?.status?.devices.find((d) => d.controller === n) || null;
}

// "Controller 1 (Logitech G29…) · Axis 3 (Z axis)" for a binding saved in
// Settings › Mods › HID Input.
function hidMenuInputName(b) {
  const d = hidMenuController(b.controller);
  const i = b.input.index;
  const usage = b.input.kind === "axis" ? HID_USAGES[d?.axes?.[i]] : null;
  const what = b.input.kind === "axis" ? `Axis ${i + 1}${usage ? ` (${usage})` : ""}` : b.input.kind === "button" ? `Button ${i + 1}` : `Hat ${i + 1}`;
  return `Controller ${b.controller}${d ? ` (${d.name || d.id})` : ""} · ${what}`;
}

// Long device names cut to fit a callout: "Logitech G29 Dr…: Z axis".
function hidShort(text, max) {
  if (text.length <= max) return text;
  const cut = text.lastIndexOf(": ");
  if (cut > 0 && text.length - cut < max - 4) {
    const tail = text.slice(cut);
    return `${text.slice(0, max - tail.length - 1)}…${tail}`;
  }
  return `${text.slice(0, max - 1)}…`;
}

function hidAgo(s) {
  if (s < 90) return `${s} s ago`;
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 172800) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

function hidPluginRows() {
  const s = hidState;
  const i = s.installed;
  const plugin = i
    ? [`${i.version || "?"} installed`, i.enabled ? "" : " · disabled", " ",
      el("span", { class: `badge ${i.current ? "ok" : "warn"}` }, i.current ? "matches this CPMX2077 build" : `differs from the ${s.bundled || "?"} in this CPMX2077 build`)]
    : s.unmanaged ? "not managed: the DLL is in the plugin folder but was not installed by CPMX2077" : el("span", { class: "muted" }, "not installed");
  const xinput = s.game_imports.filter((x) => x.toLowerCase().startsWith("xinput"));
  const imports = s.game_imports.length
    ? [el("span", { class: `badge ${xinput.length ? "ok" : "warn"}` }, xinput.length ? "XInput imported" : "no XInput import"), " ", el("span", { class: "mono small" }, s.game_imports.join(" · "))]
    : el("span", { class: "muted" }, "no XInput, DirectInput or GameInput import found in bin/x64/Cyberpunk2077.exe (read from its import table; a DLL loaded at run time is not listed)");
  const st = s.status;
  let run = el("span", { class: "muted" }, "none recorded: the plugin writes status.json while the game runs");
  if (st) {
    run = el("div", {},
      el("div", {}, `${hidAgo(s.status_age)} · plugin ${st.plugin_version} · ${st.hooks.length} XInput hooks · `,
        `XInputGetState calls: ${st.get_state_calls.toLocaleString()} · reads with bound input: ${st.merged_reads.toLocaleString()}`,
        st.emulated_reads ? ` · reported gamepad: ${st.emulated_reads.toLocaleString()}` : "",
        ` · bindings active: ${st.active_bindings} of ${st.bindings}`),
      st.get_state_calls === 0 && st.hooks.length ? el("div", { class: "small" }, el("span", { class: "badge warn" }, "no XInput reads"),
        " The game did not call XInputGetState during this run; bound inputs reach it only through that call.") : null,
      st.missing.length ? el("div", { class: "small" }, el("span", { class: "badge warn" }, "not connected"), ` ${st.missing.join(", ")}`) : null,
      st.config_error ? el("div", { class: "small" }, el("span", { class: "badge bad" }, "bindings file"), ` ${st.config_error}`) : null,
      st.hook_errors.length ? el("div", { class: "small" }, el("span", { class: "badge bad" }, "hooks"), ` ${st.hook_errors.join("; ")}`) : null,
      st.devices.length ? el("div", { class: "small" }, "Controllers the plugin read in the game:",
        el("ul", { class: "hid-run-devices" }, st.devices.map((x) => el("li", {},
          x.controller ? el("b", {}, `Controller ${x.controller}: `) : null,
          `${x.name || x.id} `, el("span", { class: "mono muted" }, x.id),
          x.xinput ? el("span", { class: "muted" }, " · XInput gamepad, read by the game itself") : null,
          x.bound ? [" ", el("span", { class: "badge ok" }, "bound")] : null,
          x.axes.length ? el("div", { class: "muted" }, x.axes.map((u, i) => `Axis ${i + 1}: ${HID_USAGES[u] || `usage ${u.toString(16)}`}`).join(" · "),
            ` · ${x.buttons} button${x.buttons === 1 ? "" : "s"}${x.hats ? ` · ${x.hats} hat${x.hats === 1 ? "" : "s"}` : ""}`) : null)))) : null);
  }
  let menu;
  if (!s.mod_settings) {
    menu = [el("span", { class: "badge warn" }, "Mod Settings not installed"),
      " Settings › Mods › HID Input in the game needs the Mod Settings mod (it uses RED4ext, redscript and ArchiveXL). ",
      el("button", { class: "small", title: "Queue Mod Settings from Nexus Mods (mod 4885)",
        onclick: (e) => busy(e.target, () => enqueue([{ kind: "nexus", name: "Mod Settings", modId: 4885 }])) }, "Get Mod Settings")];
  } else if (s.menu_error) {
    menu = [el("span", { class: "badge bad" }, "user.ini"), ` ${s.menu_error}`];
  } else {
    const m = s.menu;
    menu = [el("span", { class: "badge ok" }, "Mod Settings installed"), s.redscript ? null : [" ", el("span", { class: "badge warn" }, "redscript missing")], " ",
      m ? `${m.bindings.length} binding${m.bindings.length === 1 ? "" : "s"} saved in Settings › Mods › HID Input · bindings ${m.enabled ? "on" : "off"} · slot ${m.slot + 1} · gamepad ${m.emulate_pad ? "reported" : "not reported"} when none is connected`
        : el("span", { class: "muted" }, "nothing saved there yet (read from red4ext/plugins/mod_settings/user.ini)")];
  }
  return el("table", { class: "req-table" }, el("tbody", {},
    el("tr", {}, el("td", {}, "Plugin"), el("td", {}, plugin)),
    el("tr", {}, el("td", {}, "RED4ext"), el("td", {}, s.red4ext ? el("span", { class: "badge ok" }, "installed")
      : [el("span", { class: "badge bad" }, "missing"), " The plugin loads through RED4ext: Game panel › Frameworks."])),
    el("tr", {}, el("td", {}, "Game input imports"), el("td", {}, imports)),
    el("tr", {}, el("td", {}, "In-game menu"), el("td", {}, menu)),
    el("tr", {}, el("td", {}, "Last game run"), el("td", {}, run))));
}

function hidOptions() {
  const set = (k, v) => { hidBind[k] = v; hidDirty = true; renderHid(); };
  // Once saved in the game menu, its general settings are the ones the plugin uses.
  const menu = hidState.menu;
  const off = menu ? "" : null;
  const why = menu ? "Set in the game: Settings › Mods › HID Input" : null;
  return el("div", { class: "row hid-options" },
    el("label", { class: "check", title: why || "Off: the plugin passes the gamepad through unchanged" },
      el("input", { type: "checkbox", disabled: off, checked: (menu ? menu.enabled : hidBind.enabled) ? "" : null, onchange: (e) => set("enabled", e.target.checked) }), " Bindings on"),
    el("label", { class: "row small", title: why || "The XInput slot the bound inputs go to; 1 is the first gamepad" }, "Gamepad slot",
      el("select", { class: "inline", disabled: off, onchange: (e) => set("slot", Number(e.target.value)) },
        [0, 1, 2, 3].map((n) => el("option", { value: n, selected: (menu ? menu.slot : hidBind.slot) === n ? "" : null }, String(n + 1))))),
    el("label", { class: "check", title: why || "With no gamepad on the slot, the plugin reports one there while a bound device is connected, so bound devices work next to mouse and keyboard" },
      el("input", { type: "checkbox", disabled: off, checked: (menu ? menu.emulate_pad : hidBind.emulate_pad) ? "" : null, onchange: (e) => set("emulate_pad", e.target.checked) }),
      " Report a gamepad when none is connected"),
    menu ? el("span", { class: "muted small" }, "set in the game menu") : null,
    el("span", { class: "spacer" }),
    hidDirty ? el("span", { class: "badge warn" }, "unsaved changes") : null,
    el("button", { class: hidDirty ? "primary" : null, disabled: hidDirty ? null : "",
      title: "Write bindings.json to the plugin folder; the plugin reads it within a second, also in a running game",
      onclick: (e) => busy(e.target, async () => {
        const { gameId, ...b } = hidBind;
        await invoke("hid_save_bindings", { gameId: currentGame.id, bindings: b });
        hidDirty = false;
        toast(`Saved ${b.bindings.length} binding${b.bindings.length === 1 ? "" : "s"} to ${hidState.plugin_dir}`);
        await loadHid();
      }) }, "Save bindings"));
}

function hidDeviceCard(d) {
  const pic = drawDevice(d);
  hidPictures.set(d.id, pic.update);
  const bars = d.axes.map((a, i) => {
    const bar = el("span", { class: "hid-bar-fill" });
    hidPictures.set(`${d.id}/a${i}`, (l) => { bar.style.width = `${Math.round((l?.axes[i] ?? 0) * 100)}%`; });
    const bound = hidBind.bindings.filter((b) => b.device === d.id && b.input.kind === "axis" && b.input.index === i);
    return el("div", { class: "hid-axis" }, el("span", { class: "small" }, a.label), el("span", { class: "hid-bar" }, bar),
      el("span", { class: "small muted" }, bound.map((b) => HID_TARGET[b.target].label).join(", ")));
  });
  const lamps = [];
  for (let i = 0; i < Math.min(d.buttons, 64); i++) {
    const lamp = el("span", { class: "hid-btn", title: `Button ${i + 1}` }, String(i + 1));
    lamps.push(lamp);
  }
  for (let h = 0; h < d.hats; h++) lamps.push(el("span", { class: "hid-btn hat", title: `Hat ${h + 1}`, "data-hat": h }, `H${h + 1}`));
  hidPictures.set(`${d.id}/buttons`, (l) => {
    lamps.forEach((x, i) => {
      if (x.dataset.hat !== undefined) x.classList.toggle("on", l?.hats[Number(x.dataset.hat)] !== null && l?.hats[Number(x.dataset.hat)] !== undefined);
      else x.classList.toggle("on", !!l?.buttons[i]);
    });
  });
  return el("div", { class: "hid-device" },
    el("div", { class: "row" }, el("b", {}, d.name || "Unnamed device"), el("span", { class: "muted small mono" }, d.id),
      hidState.status?.devices.find((x) => x.id === d.id && x.controller)
        ? el("span", { class: "badge", title: "Its number in Settings › Mods › HID Input, from the plugin's last game run" },
          `Controller ${hidState.status.devices.find((x) => x.id === d.id).controller}`) : null,
      el("span", { class: "spacer" }),
      el("select", { class: "inline small", "aria-label": "Drawing", title: "Which drawing to show for this device",
        onchange: (e) => { hidKinds[d.id] = e.target.value; savePref("hidKinds", hidKinds); renderHid(); } },
        Object.entries(HID_KINDS).map(([k, label]) => el("option", { value: k, selected: hidKindOf(d) === k ? "" : null }, label)))),
    pic.svg,
    d.note ? el("p", { class: "small" }, el("span", { class: "badge bad" }, "no live values"), ` ${d.note}`) : null,
    bars.length ? el("div", { class: "hid-axes" }, bars) : null,
    lamps.length ? el("div", { class: "hid-btns" }, lamps) : null);
}

function hidBindingsTable() {
  hidRows.length = 0;
  if (!hidBind.bindings.length) return el("p", { class: "muted" }, "No bindings yet. Click a target on the gamepad, then move or press the input.");
  const edit = (b, k, v) => { b[k] = v; hidDirty = true; renderHid(); };
  return el("table", { class: "hid-bindings" },
    el("thead", {}, el("tr", {}, el("th", {}, "Input"), el("th", {}, "Gamepad"), el("th", { title: "Flip the input's direction" }, "Invert"),
      el("th", { title: "Travel ignored at rest (pedals, triggers) or around the middle (whole stick axes)" }, "Deadzone"),
      el("th", { title: "Linear: output follows travel. Progressive and Strong: finer near rest" }, "Response"),
      el("th", { title: "Which part of a stick axis the input drives" }, "Stick range"), el("th", {}, "Output"), el("th", {}))),
    el("tbody", {}, hidBind.bindings.map((b) => {
      const t = HID_TARGET[b.target];
      const digital = t.kind === "button" || t.kind === "hat";
      const out = digital ? el("span", { class: "hid-btn" }, "●") : el("span", { class: "hid-bar-fill" });
      const outCell = digital ? out : el("span", { class: `hid-bar${t.kind === "stick" ? " centred" : ""}` }, out);
      hidRows.push({ b, out, t });
      const connected = hidDevices.some((d) => d.id === b.device);
      const analog = b.input.kind !== "hat" && t.kind !== "hat";
      return el("tr", {},
        el("td", {}, hidInputName(b), connected ? null : [" ", el("span", { class: "badge warn" }, "not connected")]),
        el("td", {}, t.label),
        el("td", {}, analog ? el("input", { type: "checkbox", checked: b.invert ? "" : null, onchange: (e) => edit(b, "invert", e.target.checked) }) : null),
        el("td", {}, analog && b.input.kind === "axis" ? el("input", { type: "range", min: 0, max: 0.5, step: 0.01, value: b.deadzone, class: "hid-range",
          title: `${Math.round(b.deadzone * 100)} %`, onchange: (e) => edit(b, "deadzone", Number(e.target.value)) }) : null),
        el("td", {}, analog && b.input.kind === "axis" && t.kind !== "button" ? el("select", { class: "inline small", onchange: (e) => edit(b, "curve", Number(e.target.value)) },
          HID_CURVES.map(([v, l]) => el("option", { value: v, selected: Math.abs((b.curve || 1) - v) < 0.01 ? "" : null }, l))) : null),
        el("td", {}, t.kind === "stick" ? el("select", { class: "inline small", onchange: (e) => edit(b, "half", e.target.value) },
          HID_HALVES.filter(([v]) => v !== "full" || b.input.kind === "axis").map(([v, l]) => el("option", { value: v, selected: (b.half || "full") === v ? "" : null }, l))) : null),
        el("td", {}, outCell),
        el("td", { class: "actions" }, el("button", { class: "link", title: "Remove this binding",
          onclick: () => { hidBind.bindings.splice(hidBind.bindings.indexOf(b), 1); hidDirty = true; renderHid(); } }, "Remove")));
    })));
}

// Bindings saved in Settings › Mods › HID Input: shown, changed in the game.
function hidMenuTable() {
  const m = hidState.menu;
  if (!m?.bindings.length) return null;
  const pct = (v) => `${Math.round(v * 100)} %`;
  const curve = (c) => HID_CURVES.find(([v]) => Math.abs(v - c) < 0.01)?.[1] || String(c);
  return [el("h3", {}, "Set in the game menu"),
    el("p", { class: "muted small" }, "Saved in Settings › Mods › HID Input (Mod Settings' user.ini) and used together with the bindings above. Change them in the game."),
    el("table", { class: "hid-bindings" },
      el("thead", {}, el("tr", {}, ["Input", "Gamepad", "Invert", "Deadzone", "Response", "Stick range"].map((h) => el("th", {}, h)))),
      el("tbody", {}, m.bindings.map((b) => {
        const t = HID_TARGET[b.target];
        const analog = t.kind === "trigger" || t.kind === "stick";
        return el("tr", {},
          el("td", {}, hidMenuInputName(b), hidMenuController(b.controller) || !hidState.status ? null : [" ", el("span", { class: "badge warn" }, "not connected in the last game run")]),
          el("td", {}, t.label),
          el("td", {}, analog ? (b.invert ? "yes" : "no") : ""),
          el("td", {}, analog ? pct(b.deadzone) : ""),
          el("td", {}, analog ? curve(b.curve) : ""),
          el("td", {}, t.kind === "stick" ? HID_HALVES.find(([v]) => v === b.half)?.[1] || b.half : ""));
      })))];
}

function renderHid() {
  const box = $("#hid");
  if (!hidState || !hidBind) return;
  hidPictures.clear();
  const s = hidState;
  const i = s.installed;
  const installBtn = s.bundled && (!i || (i.enabled && !i.current))
    ? el("button", { class: "primary", title: "Install the plugin DLL this CPMX2077 build carries into red4ext/plugins/cpmx_hid_input as a tracked mod",
      onclick: (e) => busy(e.target, async () => {
        reportInstall(await invoke("hid_install", { gameId: currentGame.id }), "Takes effect at the next game start.");
        await loadMods();
      }) }, i ? `Update plugin to ${s.bundled}` : "Install plugin")
    : null;
  const pad = drawGamepad(true);
  hidPadUpdate = pad.update;
  box.replaceChildren(...[
    el("p", { class: "muted" }, "CPMX2077 HID Input is a RED4ext plugin carried in CPMX2077. In the game it reads the controllers bound here ",
      "(pedals, wheels, flight sticks, button boxes) and adds them to the XInput gamepad Cyberpunk 2077 reads: triggers take the larger value, ",
      "stick axes add up, buttons combine. Works next to a real gamepad, or alone. Bindings can also be set in the game, ",
      "under Settings › Mods › HID Input (needs the Mod Settings mod); both apply."),
    hidPluginRows(),
    el("div", { class: "row" }, installBtn,
      s.bundled ? null : el("span", { class: "muted small" }, "This CPMX2077 build carries no plugin DLL (built without CPMX_HID_PLUGIN_DLL)."),
      el("button", { title: "Open red4ext/plugins/cpmx_hid_input",
        onclick: (e) => busy(e.target, () => invoke("open_game_folder", { gameId: currentGame.id, path: "red4ext/plugins/cpmx_hid_input" })) }, "Open plugin folder")),
    el("h3", {}, "Bindings"),
    hidOptions(),
    hidListen ? el("p", { class: "hid-listen", "aria-live": "polite" }, `Move or press the input for ${HID_TARGET[hidListen.target].label}. Esc cancels.`) : null,
    pad.svg,
    hidBindingsTable(),
    ...(hidMenuTable() || []),
    el("h3", {}, "Controllers on this computer"),
    el("div", { class: "row" },
      el("span", { class: "muted small" }, navigator.userAgent.includes("Windows")
        ? "Read through hid.dll, as the plugin reads them in the game."
        : "Read from /dev/input; Proton passes the same devices to the game. Live values need read access to the device node."),
      el("span", { class: "spacer" }),
      el("button", { title: "List the connected game controllers again", onclick: (e) => busy(e.target, async () => { hidDevices = await invoke("hid_devices"); renderHid(); }) }, "Rescan")),
    hidDevices.length ? el("div", { class: "hid-devices" }, hidDevices.map(hidDeviceCard))
      : el("p", { class: "muted" }, "No game controller found."),
  ].filter(Boolean));
  hidTick();
}

// Live values into the drawings, the gamepad and the bindings table; and
// the input a listening target waits for.
function hidTick() {
  if (hidListen) {
    const found = hidDetect();
    if (found) {
      hidAddBinding(found);
      hidListen = null;
      renderHid();
      return;
    }
  }
  for (const d of hidDevices) {
    const l = hidLive.get(d.id);
    hidPictures.get(d.id)?.(l);
    hidPictures.get(`${d.id}/buttons`)?.(l);
    d.axes.forEach((_, i) => hidPictures.get(`${d.id}/a${i}`)?.(l));
  }
  hidPadUpdate?.(hidContribution());
  for (const r of hidRows) {
    const o = hidOutput(r.b);
    const v = o?.value ?? 0;
    if (r.t.kind === "stick") {
      r.out.style.left = `${Math.round(50 + Math.min(0, v) * 50)}%`;
      r.out.style.width = `${Math.round(Math.abs(v) * 50)}%`;
    } else if (o?.bits !== undefined) {
      r.out.classList.toggle("on", o.bits !== 0);
    } else {
      r.out.style.width = `${Math.round(v * 100)}%`;
    }
  }
}
