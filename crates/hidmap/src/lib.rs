//! Bindings for the CPMX2077 HID Input RED4ext plugin, shared by the plugin
//! (inside the game) and CPMX2077 (which writes the bindings file).
//!
//! Cyberpunk 2077 reads one XInput gamepad. Each binding takes one input of a
//! HID game controller (an axis, a button or a hat) and adds it to that
//! gamepad: triggers take the larger value, stick axes add up, buttons
//! combine. Pedals, wheels, sticks and button boxes then drive the game next
//! to (or instead of) a real gamepad.
//!
//! Inputs are stored so they are found again whichever way the device reaches
//! the game. Windows and Proton's hidraw path keep the device's own HID
//! descriptor, so axes are matched by HID usage; Proton's SDL path rebuilds
//! the descriptor with axes in evdev code order, so axes are matched by
//! position. [`resolve`] picks between the two.

pub mod descriptor;
#[cfg(windows)]
pub mod win;

use serde::{Deserialize, Serialize};

pub const PLUGIN_NAME: &str = "CPMX2077 HID Input";
/// The plugin's version, reported to RED4ext and in `status.json`; raise it
/// with every change to the plugin so CPMX2077 offers the update.
pub const PLUGIN_VERSION: &str = "0.1.0";
/// Plugin folder under the game root. RED4ext loads
/// `red4ext/plugins/<folder>/<name>.dll`, one folder level deep.
pub const PLUGIN_DIR: &str = "red4ext/plugins/cpmx_hid_input";
pub const PLUGIN_DLL: &str = "cpmx_hid_input.dll";
/// Written by CPMX2077, read by the plugin (again whenever it changes).
pub const BINDINGS_FILE: &str = "bindings.json";
/// Written by the plugin while the game runs, read by CPMX2077.
pub const STATUS_FILE: &str = "status.json";

pub const PAGE_GENERIC: u16 = 0x01;
pub const PAGE_SIMULATION: u16 = 0x02;
pub const PAGE_BUTTON: u16 = 0x09;
pub const GD_HAT_SWITCH: u16 = 0x39;

/// A HID usage as one number: page in the high half, usage id in the low.
pub const fn usage(page: u16, id: u16) -> u32 {
    (page as u32) << 16 | id as u32
}

/// The axis name HID gives a usage, for labels.
pub fn usage_label(u: u32) -> Option<&'static str> {
    let (page, id) = ((u >> 16) as u16, u as u16);
    Some(match (page, id) {
        (PAGE_GENERIC, 0x30) => "X axis",
        (PAGE_GENERIC, 0x31) => "Y axis",
        (PAGE_GENERIC, 0x32) => "Z axis",
        (PAGE_GENERIC, 0x33) => "X rotation",
        (PAGE_GENERIC, 0x34) => "Y rotation",
        (PAGE_GENERIC, 0x35) => "Z rotation",
        (PAGE_GENERIC, 0x36) => "Slider",
        (PAGE_GENERIC, 0x37) => "Dial",
        (PAGE_GENERIC, 0x38) => "Wheel",
        (PAGE_SIMULATION, 0xBA) => "Rudder",
        (PAGE_SIMULATION, 0xBB) => "Throttle",
        (PAGE_SIMULATION, 0xC4) => "Accelerator",
        (PAGE_SIMULATION, 0xC5) => "Brake",
        (PAGE_SIMULATION, 0xC6) => "Clutch",
        (PAGE_SIMULATION, 0xC8) => "Steering",
        _ => return None,
    })
}

/// Device key used by bindings: `vvvv:pppp`, plus `#n` for the n-th of
/// several identical devices.
pub fn device_id(vid: u16, pid: u16, instance: u32) -> String {
    if instance == 0 { format!("{vid:04x}:{pid:04x}") } else { format!("{vid:04x}:{pid:04x}#{instance}") }
}

// ---- the bindings file ------------------------------------------------------

/// `bindings.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bindings {
    #[serde(default = "one")]
    pub version: u32,
    /// Off: the plugin passes the gamepad through unchanged.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// XInput user index (0 to 3) the bound inputs go to.
    #[serde(default)]
    pub slot: u32,
    /// With no real gamepad on `slot`, report one there while a bound device
    /// is connected, so pedals work next to mouse and keyboard.
    #[serde(default = "yes")]
    pub emulate_pad: bool,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

fn one() -> u32 {
    1
}
fn yes() -> bool {
    true
}
fn linear() -> f32 {
    1.0
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings { version: 1, enabled: true, slot: 0, emulate_pad: true, devices: Vec::new(), bindings: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// [`device_id`].
    pub id: String,
    pub vid: u16,
    pub pid: u16,
    /// Which of several identical devices (same vid:pid), from 0.
    #[serde(default)]
    pub instance: u32,
    #[serde(default)]
    pub name: String,
    /// Axis usages in the device's own HID descriptor order, when known.
    #[serde(default)]
    pub hid_axes: Vec<u32>,
    /// Picture CPMX2077 draws for it (pedals, wheel, stick, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Input {
    /// `index`: position among the device's axes (evdev code order, hats
    /// left out). `usage`/`occurrence`: the axis in the device's own HID
    /// descriptor (the n-th axis with that usage), when known.
    Axis {
        index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<u32>,
        #[serde(default)]
        occurrence: u32,
    },
    /// Position among the device's buttons, from 0.
    Button { index: u32 },
    /// Position among the device's hat switches, from 0.
    Hat { index: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
    A,
    B,
    X,
    Y,
    LeftShoulder,
    RightShoulder,
    Back,
    Start,
    LeftThumb,
    RightThumb,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    /// All four directions, from a hat switch.
    Dpad,
}

impl Target {
    /// The XInput button bit, for button targets.
    pub fn button(self) -> Option<u16> {
        Some(match self {
            Target::DpadUp => 0x0001,
            Target::DpadDown => 0x0002,
            Target::DpadLeft => 0x0004,
            Target::DpadRight => 0x0008,
            Target::Start => 0x0010,
            Target::Back => 0x0020,
            Target::LeftThumb => 0x0040,
            Target::RightThumb => 0x0080,
            Target::LeftShoulder => 0x0100,
            Target::RightShoulder => 0x0200,
            Target::A => 0x1000,
            Target::B => 0x2000,
            Target::X => 0x4000,
            Target::Y => 0x8000,
            _ => return None,
        })
    }

    fn is_stick(self) -> bool {
        matches!(self, Target::LeftX | Target::LeftY | Target::RightX | Target::RightY)
    }
}

/// Which part of a stick axis an axis or button drives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Half {
    /// The whole axis: input at rest in the middle, as a wheel or stick.
    #[default]
    Full,
    /// From the middle to the positive end (right, up), as a pedal.
    Positive,
    /// From the middle to the negative end (left, down).
    Negative,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// [`Device::id`].
    pub device: String,
    pub input: Input,
    pub target: Target,
    #[serde(default)]
    pub invert: bool,
    /// Ignored travel, 0 to 0.5: at the rest end (pedals, triggers, half
    /// axes) or around the middle (full stick axes).
    #[serde(default)]
    pub deadzone: f32,
    /// Response exponent: 1 linear, above 1 finer near rest.
    #[serde(default = "linear")]
    pub curve: f32,
    #[serde(default)]
    pub half: Half,
}

// ---- matching inputs on the device the game sees ----------------------------

/// The inputs of a connected device as the reading side lists them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// Axis usages in descriptor order.
    pub axes: Vec<u32>,
    pub buttons: usize,
    pub hats: usize,
}

/// Where a binding's input sits in a [`Layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Axis(usize),
    Button(usize),
    Hat(usize),
}

fn same_set(a: &[u32], b: &[u32]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// Find `input` on a device that reports `layout`. When the device shows
/// exactly the axis usages CPMX2077 read from its own descriptor, the usage
/// decides (Windows, Proton hidraw); otherwise the descriptor was rebuilt
/// (Proton's SDL path, axes in evdev code order) and the position decides.
pub fn resolve(input: &Input, dev: &Device, layout: &Layout) -> Option<Slot> {
    match *input {
        Input::Axis { index, usage, occurrence } => {
            if let Some(u) = usage
                && !dev.hid_axes.is_empty()
                && same_set(&dev.hid_axes, &layout.axes)
            {
                return layout
                    .axes
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| **a == u)
                    .nth(occurrence as usize)
                    .map(|(i, _)| Slot::Axis(i));
            }
            ((index as usize) < layout.axes.len()).then_some(Slot::Axis(index as usize))
        }
        Input::Button { index } => ((index as usize) < layout.buttons).then_some(Slot::Button(index as usize)),
        Input::Hat { index } => ((index as usize) < layout.hats).then_some(Slot::Hat(index as usize)),
    }
}

// ---- merging into the gamepad -------------------------------------------------

/// Current values of a connected device: axes 0 to 1 over their logical
/// range, hats as one of 8 directions clockwise from up (0) or centred.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    pub axes: Vec<f32>,
    pub buttons: Vec<bool>,
    pub hats: Vec<Option<u8>>,
}

/// A binding found on a connected device.
#[derive(Debug, Clone, PartialEq)]
pub struct Active {
    pub binding: Binding,
    /// Index into the connected devices' [`Live`] list.
    pub device: usize,
    pub slot: Slot,
}

/// The XInput gamepad fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pad {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

/// What the bound inputs add: triggers 0 to 1, stick axes -1 to 1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Contribution {
    pub buttons: u16,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
}

impl Contribution {
    pub fn is_neutral(&self) -> bool {
        *self == Contribution::default()
    }

    /// `real` with these inputs added: triggers take the larger value,
    /// stick axes add up (clamped), buttons combine.
    pub fn apply(&self, real: Pad) -> Pad {
        let trig = |r: u8, c: f32| r.max((c.clamp(0.0, 1.0) * 255.0).round() as u8);
        let stick = |r: i16, c: f32| (r as i32 + (c.clamp(-1.0, 1.0) * 32767.0).round() as i32).clamp(-32768, 32767) as i16;
        Pad {
            buttons: real.buttons | self.buttons,
            left_trigger: trig(real.left_trigger, self.left_trigger),
            right_trigger: trig(real.right_trigger, self.right_trigger),
            lx: stick(real.lx, self.lx),
            ly: stick(real.ly, self.ly),
            rx: stick(real.rx, self.rx),
            ry: stick(real.ry, self.ry),
        }
    }
}

/// Travel from rest, 0 to 1, after invert, deadzone and curve.
fn from_rest(v: f32, b: &Binding) -> f32 {
    let v = if b.invert { 1.0 - v } else { v }.clamp(0.0, 1.0);
    let dz = b.deadzone.clamp(0.0, 0.5);
    let v = if v <= dz { 0.0 } else { (v - dz) / (1.0 - dz) };
    v.powf(curve(b))
}

/// -1 to 1 around the middle, after invert, deadzone and curve.
fn centred(v: f32, b: &Binding) -> f32 {
    let s = (v.clamp(0.0, 1.0) * 2.0 - 1.0) * if b.invert { -1.0 } else { 1.0 };
    let dz = b.deadzone.clamp(0.0, 0.5);
    let m = s.abs();
    let m = if m <= dz { 0.0 } else { (m - dz) / (1.0 - dz) };
    m.powf(curve(b)) * s.signum()
}

fn curve(b: &Binding) -> f32 {
    if b.curve.is_finite() { b.curve.clamp(0.2, 5.0) } else { 1.0 }
}

/// The 8-way hat direction as dpad bits.
fn hat_buttons(dir: Option<u8>) -> u16 {
    const UP: u16 = 0x0001;
    const DOWN: u16 = 0x0002;
    const LEFT: u16 = 0x0004;
    const RIGHT: u16 = 0x0008;
    match dir {
        Some(0) => UP,
        Some(1) => UP | RIGHT,
        Some(2) => RIGHT,
        Some(3) => DOWN | RIGHT,
        Some(4) => DOWN,
        Some(5) => DOWN | LEFT,
        Some(6) => LEFT,
        Some(7) => UP | LEFT,
        _ => 0,
    }
}

/// What every active binding adds right now.
pub fn contribution(active: &[Active], live: &[Live]) -> Contribution {
    let mut c = Contribution::default();
    for a in active {
        let Some(dev) = live.get(a.device) else { continue };
        let b = &a.binding;
        // Normalise the input: axes and buttons to 0..1, hats straight to bits.
        let raw = match a.slot {
            Slot::Axis(i) => match dev.axes.get(i) {
                Some(v) => *v,
                None => continue,
            },
            Slot::Button(i) => match dev.buttons.get(i) {
                Some(p) => f32::from(u8::from(*p)),
                None => continue,
            },
            Slot::Hat(i) => {
                if b.target == Target::Dpad {
                    c.buttons |= hat_buttons(dev.hats.get(i).copied().flatten());
                }
                continue;
            }
        };
        let is_axis = matches!(a.slot, Slot::Axis(_));
        if let Some(bit) = b.target.button() {
            if from_rest(raw, b) > 0.5 {
                c.buttons |= bit;
            }
            continue;
        }
        if b.target.is_stick() {
            let v = match (b.half, is_axis) {
                (Half::Full, true) => centred(raw, b),
                (Half::Negative, _) => -from_rest(raw, b),
                _ => from_rest(raw, b),
            };
            let field = match b.target {
                Target::LeftX => &mut c.lx,
                Target::LeftY => &mut c.ly,
                Target::RightX => &mut c.rx,
                _ => &mut c.ry,
            };
            *field = (*field + v).clamp(-1.0, 1.0);
            continue;
        }
        let v = from_rest(raw, b);
        match b.target {
            Target::LeftTrigger => c.left_trigger = c.left_trigger.max(v),
            Target::RightTrigger => c.right_trigger = c.right_trigger.max(v),
            _ => {}
        }
    }
    c
}

// ---- plugin status ------------------------------------------------------------

/// `status.json`: what the plugin found in the running game.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub plugin_version: String,
    /// Unix seconds.
    pub started: u64,
    pub updated: u64,
    /// `module!function` for every XInput function hooked.
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub hook_errors: Vec<String>,
    /// XInputGetState calls by the game, any slot.
    #[serde(default)]
    pub get_state_calls: u64,
    /// Reads of the bound slot that carried bound input.
    #[serde(default)]
    pub merged_reads: u64,
    /// Reads answered with a reported gamepad (no real one on the slot).
    #[serde(default)]
    pub emulated_reads: u64,
    #[serde(default)]
    pub config_error: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bindings: usize,
    /// Bindings whose device and input were found.
    #[serde(default)]
    pub active_bindings: usize,
    /// Game controllers the plugin can read.
    #[serde(default)]
    pub devices: Vec<StatusDevice>,
    /// Bound devices not connected.
    #[serde(default)]
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusDevice {
    pub id: String,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub axes: Vec<u32>,
    pub buttons: usize,
    pub hats: usize,
    pub bound: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAS: u32 = usage(PAGE_SIMULATION, 0xC4);
    const BRAKE: u32 = usage(PAGE_SIMULATION, 0xC5);
    const X: u32 = usage(PAGE_GENERIC, 0x30);
    const Y: u32 = usage(PAGE_GENERIC, 0x31);
    const Z: u32 = usage(PAGE_GENERIC, 0x32);
    const RX: u32 = usage(PAGE_GENERIC, 0x33);
    const RZ: u32 = usage(PAGE_GENERIC, 0x35);

    fn dev(hid_axes: Vec<u32>) -> Device {
        Device { id: "046d:c24f".into(), vid: 0x046d, pid: 0xc24f, instance: 0, name: "Pedals".into(), hid_axes, kind: None }
    }

    fn bind(input: Input, target: Target) -> Binding {
        Binding { device: "046d:c24f".into(), input, target, invert: false, deadzone: 0.0, curve: 1.0, half: Half::Full }
    }

    #[test]
    fn resolves_by_usage_when_the_descriptor_is_the_devices_own() {
        // Wheel: X steering, Y clutch, Z gas, Rz brake (hidraw or Windows).
        let d = dev(vec![X, Y, Z, RZ]);
        let layout = Layout { axes: vec![X, Y, Z, RZ], buttons: 0, hats: 0 };
        let brake = Input::Axis { index: 3, usage: Some(RZ), occurrence: 0 };
        assert_eq!(resolve(&brake, &d, &layout), Some(Slot::Axis(3)));
        // Same set, other order (Windows lists by data index): usage wins.
        let layout = Layout { axes: vec![RZ, X, Y, Z], buttons: 0, hats: 0 };
        assert_eq!(resolve(&brake, &d, &layout), Some(Slot::Axis(0)));
    }

    #[test]
    fn falls_back_to_position_on_a_rebuilt_descriptor() {
        // Proton's SDL path renames the four axes X, Y, Z, Rx in evdev order.
        let d = dev(vec![X, Y, Z, RZ]);
        let layout = Layout { axes: vec![X, Y, Z, RX], buttons: 0, hats: 0 };
        let brake = Input::Axis { index: 3, usage: Some(RZ), occurrence: 0 };
        assert_eq!(resolve(&brake, &d, &layout), Some(Slot::Axis(3)));
        // Simulation usages become generic ones too.
        let d = dev(vec![GAS, BRAKE]);
        let layout = Layout { axes: vec![X, Y], buttons: 0, hats: 0 };
        assert_eq!(resolve(&Input::Axis { index: 1, usage: Some(BRAKE), occurrence: 0 }, &d, &layout), Some(Slot::Axis(1)));
        // Unknown descriptor: position only, and out of range is none.
        let d = dev(vec![]);
        assert_eq!(resolve(&Input::Axis { index: 5, usage: None, occurrence: 0 }, &d, &layout), None);
        assert_eq!(resolve(&Input::Button { index: 0 }, &d, &layout), None);
    }

    #[test]
    fn duplicate_usages_use_the_occurrence() {
        let d = dev(vec![Z, Z]);
        let layout = Layout { axes: vec![Z, Z], buttons: 2, hats: 1 };
        assert_eq!(resolve(&Input::Axis { index: 0, usage: Some(Z), occurrence: 1 }, &d, &layout), Some(Slot::Axis(1)));
        assert_eq!(resolve(&Input::Hat { index: 0 }, &d, &layout), Some(Slot::Hat(0)));
    }

    fn active(b: Binding, slot: Slot) -> Active {
        Active { binding: b, device: 0, slot }
    }

    #[test]
    fn pedals_drive_the_triggers() {
        let live = vec![Live { axes: vec![0.5, 1.0], buttons: vec![], hats: vec![] }];
        let gas = active(bind(Input::Axis { index: 0, usage: None, occurrence: 0 }, Target::RightTrigger), Slot::Axis(0));
        // This brake pedal rests at the high end.
        let mut brake = bind(Input::Axis { index: 1, usage: None, occurrence: 0 }, Target::LeftTrigger);
        brake.invert = true;
        let c = contribution(&[gas, active(brake, Slot::Axis(1))], &live);
        assert_eq!(c.right_trigger, 0.5);
        assert_eq!(c.left_trigger, 0.0);
        let pad = c.apply(Pad { right_trigger: 200, left_trigger: 10, ..Pad::default() });
        assert_eq!(pad.right_trigger, 200, "the larger trigger value wins");
        assert_eq!(pad.left_trigger, 10);
        assert_eq!(c.apply(Pad::default()).right_trigger, 128);
    }

    #[test]
    fn deadzone_and_curve() {
        let mut b = bind(Input::Axis { index: 0, usage: None, occurrence: 0 }, Target::RightTrigger);
        b.deadzone = 0.1;
        assert_eq!(from_rest(0.05, &b), 0.0);
        assert!((from_rest(0.55, &b) - 0.5).abs() < 1e-6);
        assert_eq!(from_rest(1.0, &b), 1.0);
        b.curve = 2.0;
        assert!((from_rest(0.55, &b) - 0.25).abs() < 1e-6);
        // Full stick axis: deadzone around the middle.
        let mut s = bind(Input::Axis { index: 0, usage: None, occurrence: 0 }, Target::LeftX);
        s.deadzone = 0.1;
        assert_eq!(centred(0.52, &s), 0.0);
        assert_eq!(centred(0.0, &s), -1.0);
        s.invert = true;
        assert_eq!(centred(0.0, &s), 1.0);
    }

    #[test]
    fn sticks_add_and_buttons_combine() {
        let live = vec![Live { axes: vec![1.0, 0.75], buttons: vec![true, false], hats: vec![Some(1)] }];
        let wheel = active(bind(Input::Axis { index: 0, usage: None, occurrence: 0 }, Target::LeftX), Slot::Axis(0));
        let mut pedal = bind(Input::Axis { index: 1, usage: None, occurrence: 0 }, Target::LeftY);
        pedal.half = Half::Negative;
        let a = active(bind(Input::Button { index: 0 }, Target::A), Slot::Button(0));
        let b = active(bind(Input::Button { index: 1 }, Target::B), Slot::Button(1));
        let hat = active(bind(Input::Hat { index: 0 }, Target::Dpad), Slot::Hat(0));
        let c = contribution(&[wheel, active(pedal, Slot::Axis(1)), a, b, hat], &live);
        assert_eq!(c.lx, 1.0);
        assert_eq!(c.ly, -0.75);
        assert_eq!(c.buttons, 0x1000 | 0x0001 | 0x0008);
        let pad = c.apply(Pad { lx: 20000, buttons: 0x2000, ..Pad::default() });
        assert_eq!(pad.lx, 32767, "clamped");
        assert_eq!(pad.buttons, 0x1000 | 0x2000 | 0x0001 | 0x0008);
        assert!(Contribution::default().is_neutral());
        assert_eq!(Contribution::default().apply(pad), pad);
    }

    #[test]
    fn a_button_on_a_stick_pushes_one_way() {
        let live = vec![Live { axes: vec![], buttons: vec![true], hats: vec![] }];
        let mut b = bind(Input::Button { index: 0 }, Target::RightY);
        b.half = Half::Negative;
        assert_eq!(contribution(&[active(b.clone(), Slot::Button(0))], &live).ry, -1.0);
        b.half = Half::Full;
        assert_eq!(contribution(&[active(b, Slot::Button(0))], &live).ry, 1.0);
    }

    #[test]
    fn bindings_file_round_trip_and_defaults() {
        let parsed: Bindings = serde_json::from_str(r#"{"devices":[],"bindings":[{"device":"046d:c24f","input":{"kind":"axis","index":2,"usage":131268},"target":"right_trigger"}]}"#).unwrap();
        assert!(parsed.enabled && parsed.emulate_pad);
        assert_eq!(parsed.bindings[0].curve, 1.0);
        assert_eq!(parsed.bindings[0].input, Input::Axis { index: 2, usage: Some(GAS), occurrence: 0 });
        let text = serde_json::to_string(&parsed).unwrap();
        assert_eq!(serde_json::from_str::<Bindings>(&text).unwrap(), parsed);
        assert_eq!(device_id(0x46d, 0xc24f, 0), "046d:c24f");
        assert_eq!(device_id(0x46d, 0xc24f, 1), "046d:c24f#1");
    }
}
