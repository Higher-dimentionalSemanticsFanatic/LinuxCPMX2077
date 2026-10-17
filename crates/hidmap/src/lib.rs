//! The CPMX2077 HID Input RED4ext plugin's logic that does not need the
//! game or Windows: bindings, the merge rules, the in-game menu ([`menu`])
//! and the status file. The plugin (`crates/hid-plugin`) supplies the
//! hooks and device reads; this crate is tested on any platform.
//!
//! Cyberpunk 2077 reads one XInput gamepad. Each binding takes one input of a
//! HID game controller (an axis, a button or a hat) and adds it to that
//! gamepad: triggers take the larger value, stick axes add up, buttons
//! combine. Pedals, wheels, sticks and button boxes then drive the game next
//! to (or instead of) a real gamepad.

pub mod menu;
#[cfg(windows)]
pub mod win;

use serde::Serialize;

pub const PLUGIN_NAME: &str = "CPMX2077 HID Input";
/// The plugin's version, reported to RED4ext, in `status.json` and in the
/// mod archive's name.
pub const PLUGIN_VERSION: &str = "0.3.0";
/// Plugin folder under the game root. RED4ext loads
/// `red4ext/plugins/<folder>/<name>.dll`, one folder level deep.
pub const PLUGIN_DIR: &str = "red4ext/plugins/cpmx_hid_input";
pub const PLUGIN_DLL: &str = "cpmx_hid_input.dll";
/// Written by the plugin while the game runs, for players looking up
/// controller and input numbers.
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

/// `vvvv:pppp`, plus `#n` for the n-th of several identical devices.
pub fn device_id(vid: u16, pid: u16, instance: u32) -> String {
    if instance == 0 { format!("{vid:04x}:{pid:04x}") } else { format!("{vid:04x}:{pid:04x}#{instance}") }
}

// ---- bindings ------------------------------------------------------------------

/// An input, by its position as the plugin reads the controller, from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Axis { index: u32 },
    Button { index: u32 },
    Hat { index: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Half {
    /// The whole axis: input at rest in the middle, as a wheel or stick.
    #[default]
    Full,
    /// From the middle to the positive end (right, up), as a pedal.
    Positive,
    /// From the middle to the negative end (left, down).
    Negative,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// Controller number in the in-game menu, 1 to [`menu::CONTROLLERS`].
    pub controller: u32,
    pub input: Input,
    pub target: Target,
    pub invert: bool,
    /// Ignored travel, 0 to 0.5: at the rest end (pedals, triggers, half
    /// axes) or around the middle (full stick axes).
    pub deadzone: f32,
    /// Response exponent: 1 linear, above 1 finer near rest.
    pub curve: f32,
    pub half: Half,
}

// ---- matching inputs on a connected controller ----------------------------------

/// The inputs of a connected device as the reading side lists them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// Axis usages in the order they are read.
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

/// Find `input` on a device that reports `layout`; `None` when the device
/// has fewer inputs of that kind.
pub fn resolve(input: &Input, layout: &Layout) -> Option<Slot> {
    match *input {
        Input::Axis { index } => ((index as usize) < layout.axes.len()).then_some(Slot::Axis(index as usize)),
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

// ---- finding input numbers ------------------------------------------------------

/// How many inputs [`Moves::recent`] keeps.
pub const RECENT_INPUTS: usize = 5;

/// The inputs moved on one controller, newest first, so players can look
/// up which number an input has: an axis once it travels a third of its
/// range from rest, a button when pressed, a hat when pushed. Rest is where
/// each input was in the first report read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Moves {
    rest: Option<Live>,
    away: Vec<bool>,
    pub recent: Vec<String>,
}

impl Moves {
    /// Take the controller's newest values; true when [`Moves::recent`]
    /// changed.
    pub fn update(&mut self, now: &Live) -> bool {
        let Some(rest) = &mut self.rest else {
            self.rest = Some(now.clone());
            self.away = vec![false; now.axes.len()];
            return false;
        };
        let mut moved = Vec::new();
        for (i, (v, r)) in now.axes.iter().zip(&rest.axes).enumerate() {
            let d = (v - r).abs();
            if !self.away[i] && d > 1.0 / 3.0 {
                self.away[i] = true;
                moved.push(format!("Axis {}", i + 1));
            } else if self.away[i] && d < 0.1 {
                self.away[i] = false;
            }
        }
        for (i, (b, was)) in now.buttons.iter().zip(&mut rest.buttons).enumerate() {
            if *b && !*was {
                moved.push(format!("Button {}", i + 1));
            }
            *was = *b;
        }
        for (i, (h, was)) in now.hats.iter().zip(&mut rest.hats).enumerate() {
            if h.is_some() && was.is_none() {
                moved.push(format!("Hat {}", i + 1));
            }
            *was = *h;
        }
        // Inputs moved together keep their order (axes, buttons, hats).
        for name in moved.iter().rev() {
            self.recent.retain(|r| r != name);
            self.recent.insert(0, name.clone());
        }
        self.recent.truncate(RECENT_INPUTS);
        !moved.is_empty()
    }
}

// ---- plugin status ------------------------------------------------------------

/// `status.json`: what the plugin found in the running game.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Status {
    pub plugin_version: String,
    /// Unix seconds.
    pub started: u64,
    pub updated: u64,
    /// `module!function` for every XInput function hooked.
    pub hooks: Vec<String>,
    pub hook_errors: Vec<String>,
    /// XInputGetState calls by the game, any slot.
    pub get_state_calls: u64,
    /// Reads of the bound slot that carried bound input.
    pub merged_reads: u64,
    /// Reads answered with a reported gamepad (no real one on the slot).
    pub emulated_reads: u64,
    /// Mod Settings' `user.ini` could not be read.
    pub config_error: Option<String>,
    /// Settings › Mods › HID Input values were found in `user.ini`.
    pub menu_saved: bool,
    pub enabled: bool,
    /// Bindings set in the menu.
    pub bindings: usize,
    /// Bindings whose controller and input were found.
    pub active_bindings: usize,
    /// Game controllers the plugin can read.
    pub devices: Vec<StatusDevice>,
    /// Bound controller numbers with no controller connected.
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StatusDevice {
    /// [`device_id`].
    pub id: String,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    /// Its number in the in-game menu (Controller 1 to 4).
    pub controller: Option<u32>,
    /// An XInput gamepad, read by the game itself (no number).
    pub xinput: bool,
    /// `Axis 1: X axis`, …
    pub axis_names: Vec<String>,
    /// Each axis's position now, percent of its range.
    pub axis_values: Vec<u8>,
    pub buttons: usize,
    pub hats: usize,
    /// [`Moves::recent`].
    pub recent_inputs: Vec<String>,
    /// A binding uses it.
    pub bound: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(input: Input, target: Target) -> Binding {
        Binding { controller: 1, input, target, invert: false, deadzone: 0.0, curve: 1.0, half: Half::Full }
    }

    #[test]
    fn resolves_by_position() {
        let layout = Layout { axes: vec![usage(PAGE_GENERIC, 0x30), usage(PAGE_GENERIC, 0x31)], buttons: 2, hats: 1 };
        assert_eq!(resolve(&Input::Axis { index: 1 }, &layout), Some(Slot::Axis(1)));
        assert_eq!(resolve(&Input::Axis { index: 2 }, &layout), None);
        assert_eq!(resolve(&Input::Button { index: 1 }, &layout), Some(Slot::Button(1)));
        assert_eq!(resolve(&Input::Hat { index: 0 }, &layout), Some(Slot::Hat(0)));
        assert_eq!(resolve(&Input::Hat { index: 1 }, &layout), None);
    }

    fn active(b: Binding, slot: Slot) -> Active {
        Active { binding: b, device: 0, slot }
    }

    #[test]
    fn pedals_drive_the_triggers() {
        let live = vec![Live { axes: vec![0.5, 1.0], buttons: vec![], hats: vec![] }];
        let gas = active(bind(Input::Axis { index: 0 }, Target::RightTrigger), Slot::Axis(0));
        // This brake pedal rests at the high end.
        let mut brake = bind(Input::Axis { index: 1 }, Target::LeftTrigger);
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
        let mut b = bind(Input::Axis { index: 0 }, Target::RightTrigger);
        b.deadzone = 0.1;
        assert_eq!(from_rest(0.05, &b), 0.0);
        assert!((from_rest(0.55, &b) - 0.5).abs() < 1e-6);
        assert_eq!(from_rest(1.0, &b), 1.0);
        b.curve = 2.0;
        assert!((from_rest(0.55, &b) - 0.25).abs() < 1e-6);
        // Full stick axis: deadzone around the middle.
        let mut s = bind(Input::Axis { index: 0 }, Target::LeftX);
        s.deadzone = 0.1;
        assert_eq!(centred(0.52, &s), 0.0);
        assert_eq!(centred(0.0, &s), -1.0);
        s.invert = true;
        assert_eq!(centred(0.0, &s), 1.0);
    }

    #[test]
    fn sticks_add_and_buttons_combine() {
        let live = vec![Live { axes: vec![1.0, 0.75], buttons: vec![true, false], hats: vec![Some(1)] }];
        let wheel = active(bind(Input::Axis { index: 0 }, Target::LeftX), Slot::Axis(0));
        let mut pedal = bind(Input::Axis { index: 1 }, Target::LeftY);
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
    fn moved_inputs_newest_first() {
        let live = |axes: [f32; 2], button: bool, hat: Option<u8>| Live { axes: axes.to_vec(), buttons: vec![false, button], hats: vec![hat] };
        let mut m = Moves::default();
        // Brake pedal resting at the top; held button at start doesn't count.
        assert!(!m.update(&Live { buttons: vec![false, true], ..live([0.5, 1.0], false, None) }));
        assert!(!m.update(&live([0.55, 0.95], false, None)), "noise");
        assert!(m.update(&live([0.5, 0.2], false, None)));
        assert_eq!(m.recent, ["Axis 2"]);
        assert!(!m.update(&live([0.5, 0.5], false, None)), "still away");
        assert!(!m.update(&live([0.5, 1.0], false, None)), "back at rest");
        assert!(m.update(&live([0.5, 1.0], true, Some(2))));
        assert_eq!(m.recent, ["Button 2", "Hat 1", "Axis 2"]);
        assert!(m.update(&live([0.5, 0.0], true, Some(2))));
        assert_eq!(m.recent, ["Axis 2", "Button 2", "Hat 1"], "a repeat moves to the front");
        assert_eq!(device_id(0x46d, 0xc24f, 0), "046d:c24f");
        assert_eq!(device_id(0x46d, 0xc24f, 1), "046d:c24f#1");
    }
}
