//! In-game settings: Settings › Mods › HID Input.
//!
//! The Mod Settings mod lists every script class property tagged
//! `ModSettings.mod` and saves the values to its own `user.ini`.
//! [`script`] writes that class (shipped as `HIDInput.reds`); the plugin
//! reads the saved values back with [`parse`], so no script calls into the
//! plugin are needed.
//!
//! The menu can only offer fixed choices, so it binds by number:
//! controller 1 to 4 (the plugin's [`numbered`] order) and axis, button or
//! hat 1 to n as the plugin reads that controller. `status.json` lists the
//! numbers and the inputs last moved.

use crate::{Binding, Half, Input, Target};

/// Mod Settings' saved values, relative to the game folder.
pub const USER_INI: &str = "red4ext/plugins/mod_settings/user.ini";

/// Name in Settings › Mods.
pub const MENU_NAME: &str = "HID Input";
const CLASS: &str = "CPMXHIDInputSettings";

pub const CONTROLLERS: u32 = 4;
pub const AXES: u32 = 8;
pub const BUTTONS: u32 = 32;
pub const HATS: u32 = 4;

/// A gamepad input the menu binds.
pub struct MenuTarget {
    /// Property name prefix in the script and `user.ini`.
    pub key: &'static str,
    pub target: Target,
    pub label: &'static str,
}

const fn t(key: &'static str, target: Target, label: &'static str) -> MenuTarget {
    MenuTarget { key, target, label }
}

/// Analog targets: controller, input, invert, deadzone, response (and
/// stick range on stick axes).
pub const AXIS_TARGETS: [MenuTarget; 6] = [
    t("rightTrigger", Target::RightTrigger, "Right trigger (vehicle: accelerate)"),
    t("leftTrigger", Target::LeftTrigger, "Left trigger (vehicle: brake, reverse)"),
    t("leftStickX", Target::LeftX, "Left stick X (vehicle: steer)"),
    t("leftStickY", Target::LeftY, "Left stick Y"),
    t("rightStickX", Target::RightX, "Right stick X"),
    t("rightStickY", Target::RightY, "Right stick Y"),
];

/// Button targets: controller and input.
pub const BUTTON_TARGETS: [MenuTarget; 14] = [
    t("buttonA", Target::A, "A"),
    t("buttonB", Target::B, "B"),
    t("buttonX", Target::X, "X"),
    t("buttonY", Target::Y, "Y"),
    t("leftBumper", Target::LeftShoulder, "Left bumper"),
    t("rightBumper", Target::RightShoulder, "Right bumper"),
    t("leftStickClick", Target::LeftThumb, "Left stick click"),
    t("rightStickClick", Target::RightThumb, "Right stick click"),
    t("viewButton", Target::Back, "View / Back"),
    t("menuButton", Target::Start, "Menu / Start"),
    t("dpadUp", Target::DpadUp, "D-pad up"),
    t("dpadDown", Target::DpadDown, "D-pad down"),
    t("dpadLeft", Target::DpadLeft, "D-pad left"),
    t("dpadRight", Target::DpadRight, "D-pad right"),
];

/// The whole d-pad from a hat switch.
pub const HAT_TARGET: MenuTarget = t("dpadHat", Target::Dpad, "D-pad (hat switch)");

/// Response choices and their curve exponents.
const RESPONSES: [(&str, &str, f32); 3] = [("Linear", "Linear", 1.0), ("Progressive", "Progressive", 1.6), ("Strong", "Strong", 2.5)];
const RANGES: [(&str, &str, Half); 3] = [
    ("WholeAxis", "Whole axis", Half::Full),
    ("RestToPlus", "Rest to + (right, up)", Half::Positive),
    ("RestToMinus", "Rest to - (left, down)", Half::Negative),
];

/// Values read from `user.ini`.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub enabled: bool,
    /// XInput user index, 0 to 3.
    pub slot: u32,
    pub emulate_pad: bool,
    pub bindings: Vec<Binding>,
}

impl Default for Menu {
    fn default() -> Self {
        Menu { enabled: true, slot: 0, emulate_pad: true, bindings: Vec::new() }
    }
}

/// A controller as the plugin numbers it.
#[derive(Debug, Clone, PartialEq)]
pub struct Numbered {
    pub vid: u16,
    pub pid: u16,
    pub instance: u32,
    pub name: String,
}

/// Number controllers for the menu: game controllers that are not XInput
/// gamepads, by USB vendor id, product id and instance, so the numbers
/// stay the same whatever order they were plugged in.
pub fn numbered(mut all: Vec<Numbered>) -> Vec<Numbered> {
    all.sort_by_key(|c| (c.vid, c.pid, c.instance));
    all
}

// ---- reading user.ini --------------------------------------------------------------

/// The HID Input values in Mod Settings' `user.ini`; `None` when it has
/// none (the menu was never saved). Section and key names are matched
/// without regard to case, and a key may carry its class as a prefix.
pub fn parse(ini: &str) -> Option<Menu> {
    let mut values: Vec<(String, String)> = Vec::new();
    let mut ours = false;
    for line in ini.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.to_ascii_lowercase();
            ours = name.contains(&CLASS.to_ascii_lowercase()) || name.contains(&MENU_NAME.to_ascii_lowercase());
            continue;
        }
        if !ours {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim();
        let k = k.rsplit(['.', '/', ':']).next().unwrap_or(k).to_ascii_lowercase();
        values.push((k, v.trim().trim_matches('"').to_string()));
    }
    if values.is_empty() {
        return None;
    }
    let get = |key: &str| values.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str());
    let flag = |key: &str, default: bool| get(key).map_or(default, |v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"));
    let int = |key: &str| get(key).and_then(|v| v.parse::<f64>().ok()).map(|v| v.round() as i64);

    let mut m = Menu {
        enabled: flag("enabled", true),
        slot: int("gamepadSlot").map_or(0, |s| (s.clamp(1, 4) - 1) as u32),
        emulate_pad: flag("reportGamepad", true),
        bindings: Vec::new(),
    };
    let controller = |key: &str| -> Option<u32> {
        let v = get(&format!("{key}Controller"))?;
        let n = match v.parse::<i64>() {
            Ok(n) => n,
            Err(_) => numbered_name(v, "controller")? as i64,
        };
        (1..=CONTROLLERS as i64).contains(&n).then_some(n as u32)
    };
    for mt in &AXIS_TARGETS {
        let Some(c) = controller(mt.key) else { continue };
        let input = source(get(&format!("{}Input", mt.key)), true).unwrap_or(Input::Axis { index: 0 });
        let response = choice(get(&format!("{}Response", mt.key)), &RESPONSES.map(|r| r.0)).unwrap_or(0);
        let range = choice(get(&format!("{}Range", mt.key)), &RANGES.map(|r| r.0)).unwrap_or(0);
        let mut half = RANGES[range].2;
        // A button only drives a stick from rest to one end.
        if half == Half::Full && matches!(input, Input::Button { .. }) {
            half = Half::Positive;
        }
        m.bindings.push(Binding {
            controller: c,
            input,
            target: mt.target,
            invert: flag(&format!("{}Invert", mt.key), false),
            deadzone: int(&format!("{}Deadzone", mt.key)).map_or(0.0, |p| p.clamp(0, 50) as f32 / 100.0),
            curve: RESPONSES[response].2,
            half,
        });
    }
    for mt in &BUTTON_TARGETS {
        let Some(c) = controller(mt.key) else { continue };
        let input = source(get(&format!("{}Input", mt.key)), false).unwrap_or(Input::Button { index: 0 });
        m.bindings.push(button_binding(c, input, mt.target));
    }
    if let Some(c) = controller(HAT_TARGET.key) {
        let v = get(&format!("{}Input", HAT_TARGET.key));
        let index = v.and_then(|v| v.parse::<u32>().ok().or_else(|| numbered_name(v, "hat").map(|n| n - 1))).unwrap_or(0);
        m.bindings.push(button_binding(c, Input::Hat { index: index.min(HATS - 1) }, Target::Dpad));
    }
    Some(m)
}

fn button_binding(controller: u32, input: Input, target: Target) -> Binding {
    Binding { controller, input, target, invert: false, deadzone: 0.0, curve: 1.0, half: Half::Full }
}

/// `prefixN` (any case) as N.
fn numbered_name(v: &str, prefix: &str) -> Option<u32> {
    let v = v.to_ascii_lowercase();
    v.strip_prefix(prefix)?.parse().ok()
}

/// An input choice, saved as the enum's number or its value name. Analog
/// targets list axes first, button targets buttons first.
fn source(v: Option<&str>, axes_first: bool) -> Option<Input> {
    let v = v?;
    let axis = |n: u32| (1..=AXES).contains(&n).then_some(Input::Axis { index: n - 1 });
    let button = |n: u32| (1..=BUTTONS).contains(&n).then_some(Input::Button { index: n - 1 });
    if let Ok(n) = v.parse::<u32>() {
        return if axes_first {
            if n < AXES { axis(n + 1) } else { button(n - AXES + 1) }
        } else if n < BUTTONS {
            button(n + 1)
        } else {
            axis(n - BUTTONS + 1)
        };
    }
    numbered_name(v, "axis").and_then(axis).or_else(|| numbered_name(v, "button").and_then(button))
}

/// Index of a fixed choice, saved as its number or its value name.
fn choice(v: Option<&str>, names: &[&str]) -> Option<usize> {
    let v = v?;
    match v.parse::<usize>() {
        Ok(n) => (n < names.len()).then_some(n),
        Err(_) => names.iter().position(|n| n.eq_ignore_ascii_case(v)),
    }
}

// ---- the script ------------------------------------------------------------------

/// The redscript class Mod Settings shows as Settings › Mods › HID Input.
/// Saved as `HIDInput.reds` in this crate; a test keeps the file current.
pub fn script() -> String {
    let mut s = String::new();
    s.push_str(
        "// CPMX2077 HID Input: in-game settings (Settings > Mods > HID Input).\n\
         // The Mod Settings mod shows them and saves them to\n\
         // red4ext/plugins/mod_settings/user.ini, where the HID Input RED4ext\n\
         // plugin reads them. Without Mod Settings this file does nothing.\n\
         // Generated by cp2077mm-hidmap (menu::script); edit it there.\n\n",
    );
    let mut controller = vec![("Unbound".to_string(), "None".to_string(), 0)];
    controller.extend((1..=CONTROLLERS).map(|n| (format!("Controller{n}"), format!("Controller {n}"), n as i64)));
    let axes = (1..=AXES).map(|n| (format!("Axis{n}"), format!("Axis {n}")));
    let buttons = (1..=BUTTONS).map(|n| (format!("Button{n}"), format!("Button {n}")));
    let axis_input: Vec<_> = axes.clone().chain(buttons.clone()).enumerate().map(|(i, (v, l))| (v, l, i as i64)).collect();
    let button_input: Vec<_> = buttons.chain(axes).enumerate().map(|(i, (v, l))| (v, l, i as i64)).collect();
    let hats: Vec<_> = (1..=HATS).map(|n| (format!("Hat{n}"), format!("Hat {n}"), n as i64 - 1)).collect();
    let fixed = |v: &[(&str, &str)]| -> Vec<(String, String, i64)> {
        v.iter().enumerate().map(|(i, (a, b))| (a.to_string(), b.to_string(), i as i64)).collect()
    };
    let responses = fixed(&RESPONSES.map(|r| (r.0, r.1)));
    let ranges = fixed(&RANGES.map(|r| (r.0, r.1)));
    let enums = [
        ("CPMXHIDController", &controller),
        ("CPMXHIDAxisInput", &axis_input),
        ("CPMXHIDButtonInput", &button_input),
        ("CPMXHIDHat", &hats),
        ("CPMXHIDResponse", &responses),
        ("CPMXHIDRange", &ranges),
    ];
    for (name, values) in &enums {
        s.push_str(&format!("enum {name} {{\n"));
        let lines: Vec<String> = values.iter().map(|(v, _, n)| format!("  {v} = {n}")).collect();
        s.push_str(&lines.join(",\n"));
        s.push_str("\n}\n\n");
    }
    s.push_str(&format!("public class {CLASS} {{\n"));
    let display = |values: &[(String, String, i64)]| -> Vec<(String, String)> {
        values.iter().map(|(v, l, _)| (format!("ModSettings.displayValues.{v}"), l.clone())).collect()
    };
    let range = |min: i64, max: i64| -> Vec<(String, String)> {
        vec![("ModSettings.min".into(), min.to_string()), ("ModSettings.max".into(), max.to_string()), ("ModSettings.step".into(), "1".into())]
    };
    let mut f = Field { category: "General", order: 0, desc: "", extra: Vec::new() };
    f.desc = "Off: the gamepad reaches the game unchanged.";
    f.add(&mut s, "enabled", "Bindings on", "Bool", "true");
    f.desc = "XInput slot the bound inputs go to; 1 is the first gamepad.";
    f.extra = range(1, 4);
    f.add(&mut s, "gamepadSlot", "Gamepad slot", "Int32", "1");
    f.desc = "With no gamepad on the slot, report one while a bound controller is connected, so bound controllers work without a gamepad.";
    f.extra = Vec::new();
    f.add(&mut s, "reportGamepad", "Report a gamepad when none is connected", "Bool", "true");

    let controller_desc = "Game controllers that are not XInput gamepads, numbered by USB vendor and product ID. \
                           red4ext/plugins/cpmx_hid_input/status.json lists them.";
    let input_desc = "Numbered in the order the plugin reads the controller. \
                      Move the input: status.json lists it under recent_inputs.";
    for mt in &AXIS_TARGETS {
        let k = mt.key;
        let stick = !matches!(mt.target, Target::LeftTrigger | Target::RightTrigger);
        f.category = mt.label;
        f.order += 1;
        f.desc = controller_desc;
        f.extra = display(&controller);
        f.add(&mut s, &format!("{k}Controller"), "Controller", "CPMXHIDController", "CPMXHIDController.Unbound");
        f.desc = input_desc;
        f.extra = display(&axis_input);
        f.add(&mut s, &format!("{k}Input"), "Input", "CPMXHIDAxisInput", "CPMXHIDAxisInput.Axis1");
        f.desc = "Flip the input's direction (most pedals rest at the top of their range).";
        f.extra = Vec::new();
        f.add(&mut s, &format!("{k}Invert"), "Invert", "Bool", "false");
        f.desc = "Travel ignored at rest (pedals, triggers) or around the middle (whole stick axes).";
        f.extra = range(0, 50);
        f.add(&mut s, &format!("{k}Deadzone"), "Deadzone (%)", "Int32", if stick { "5" } else { "3" });
        f.desc = "Linear: output follows travel. Progressive and Strong: finer near rest.";
        f.extra = display(&responses);
        f.add(&mut s, &format!("{k}Response"), "Response", "CPMXHIDResponse", "CPMXHIDResponse.Linear");
        if stick {
            f.desc = "Whole axis for a wheel or stick; one half for a pedal or button pushing one way.";
            f.extra = display(&ranges);
            f.add(&mut s, &format!("{k}Range"), "Stick range", "CPMXHIDRange", "CPMXHIDRange.WholeAxis");
        }
    }
    f.category = "Buttons";
    f.order += 1;
    for mt in &BUTTON_TARGETS {
        let k = mt.key;
        f.desc = controller_desc;
        f.extra = display(&controller);
        f.add(&mut s, &format!("{k}Controller"), &format!("{}: controller", mt.label), "CPMXHIDController", "CPMXHIDController.Unbound");
        f.desc = "An axis counts as pressed past half its travel.";
        f.extra = display(&button_input);
        f.add(&mut s, &format!("{k}Input"), &format!("{}: input", mt.label), "CPMXHIDButtonInput", "CPMXHIDButtonInput.Button1");
    }
    let k = HAT_TARGET.key;
    f.category = HAT_TARGET.label;
    f.order += 1;
    f.desc = controller_desc;
    f.extra = display(&controller);
    f.add(&mut s, &format!("{k}Controller"), "Controller", "CPMXHIDController", "CPMXHIDController.Unbound");
    f.desc = "All four d-pad directions follow the hat switch.";
    f.extra = display(&hats);
    f.add(&mut s, &format!("{k}Input"), "Hat switch", "CPMXHIDHat", "CPMXHIDHat.Hat1");
    s.push_str("}\n");
    s
}

/// One menu entry's category and annotations, for [`script`].
struct Field<'a> {
    category: &'a str,
    order: u32,
    desc: &'a str,
    extra: Vec<(String, String)>,
}

impl Field<'_> {
    fn add(&self, s: &mut String, name: &str, label: &str, ty: &str, default: &str) {
        let mut props = vec![
            ("ModSettings.mod".to_string(), MENU_NAME.to_string()),
            ("ModSettings.category".to_string(), self.category.to_string()),
            ("ModSettings.category.order".to_string(), self.order.to_string()),
            ("ModSettings.displayName".to_string(), label.to_string()),
        ];
        if !self.desc.is_empty() {
            props.push(("ModSettings.description".to_string(), self.desc.to_string()));
        }
        props.extend(self.extra.iter().cloned());
        s.push('\n');
        for (k, v) in props {
            s.push_str(&format!("  @runtimeProperty(\"{k}\", \"{}\")\n", v.replace('"', "'")));
        }
        s.push_str(&format!("  public let {name}: {ty} = {default};\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/reds/HIDInput.reds");

    #[test]
    fn script_file_is_current() {
        let want = script();
        if std::env::var_os("UPDATE_REDS").is_some() {
            std::fs::write(FILE, &want).unwrap();
        }
        // A checkout with CRLF line ends still counts as current.
        let have = std::fs::read_to_string(FILE).unwrap_or_default().replace("\r\n", "\n");
        assert!(have == want, "reds/HIDInput.reds is out of date: run UPDATE_REDS=1 cargo test -p cp2077mm-hidmap menu::");
    }

    #[test]
    fn every_key_the_parser_reads_is_in_the_script() {
        let s = script();
        let mut keys = vec!["enabled".to_string(), "gamepadSlot".into(), "reportGamepad".into()];
        for mt in &AXIS_TARGETS {
            for f in ["Controller", "Input", "Invert", "Deadzone", "Response"] {
                keys.push(format!("{}{f}", mt.key));
            }
        }
        keys.extend(["leftStickXRange", "rightStickYRange"].map(String::from));
        for mt in BUTTON_TARGETS.iter().chain([&HAT_TARGET]) {
            keys.push(format!("{}Controller", mt.key));
            keys.push(format!("{}Input", mt.key));
        }
        for k in keys {
            assert!(s.contains(&format!("public let {k}: ")), "{k}");
        }
        // Balanced braces: a broken script stops every redscript mod.
        assert_eq!(s.matches('{').count(), s.matches('}').count());
        assert!(!s.contains(",\n}"), "no trailing comma in enums");
    }

    #[test]
    fn nothing_saved_is_none() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("[/mods/Other Mod/OtherClass]\nenabled = false\n"), None);
    }

    #[test]
    fn reads_numbers_and_value_names_in_any_case() {
        // Enum values as numbers, mINI-style lower-case keys.
        let ini = "[/mods/HID Input/CPMXHIDInputSettings]\n\
                   enabled = true\ngamepadslot = 2\nreportgamepad = false\n\
                   righttriggercontroller = 1\nrighttriggerinput = 2\nrighttriggerinvert = true\nrighttriggerdeadzone = 10\nrighttriggerresponse = 1\n\
                   leftstickxcontroller = 2\nleftstickxinput = 0\nleftstickxrange = 0\n\
                   buttonacontroller = 1\nbuttonainput = 9\n\
                   dpadhatcontroller = 1\ndpadhatinput = 0\n";
        let m = parse(ini).unwrap();
        assert!(m.enabled && !m.emulate_pad);
        assert_eq!(m.slot, 1);
        assert_eq!(m.bindings.len(), 4);
        let rt = &m.bindings[0];
        assert_eq!((rt.controller, rt.target, rt.input), (1, Target::RightTrigger, Input::Axis { index: 2 }));
        assert!(rt.invert && (rt.deadzone - 0.1).abs() < 1e-6 && rt.curve == 1.6);
        assert_eq!((m.bindings[1].target, m.bindings[1].half), (Target::LeftX, Half::Full));
        assert_eq!(m.bindings[2].input, Input::Button { index: 9 });
        assert_eq!((m.bindings[3].target, m.bindings[3].input), (Target::Dpad, Input::Hat { index: 0 }));

        // Value names, class-prefixed keys, an axis target fed by a button.
        let ini = "[CPMXHIDInputSettings]\nCPMXHIDInputSettings.leftTriggerController=Controller2\nleftTriggerInput=Button3\n\
                   leftStickYController = Controller1\nleftStickYInput = Axis8\nleftStickYRange = RestToMinus\n\
                   rightStickXController = Unbound\nbuttonBController = Controller5\n";
        let m = parse(ini).unwrap();
        assert_eq!(m.bindings.len(), 2, "unbound and out-of-range controllers are skipped");
        assert_eq!((m.bindings[0].controller, m.bindings[0].input), (2, Input::Button { index: 2 }));
        assert_eq!((m.bindings[1].input, m.bindings[1].half), (Input::Axis { index: 7 }, Half::Negative));
        assert!(m.enabled && m.emulate_pad && m.slot == 0, "defaults for unsaved values");
    }

    #[test]
    fn numbers_controllers_by_vendor_and_product() {
        let pedals = Numbered { vid: 0x06a3, pid: 0x0763, instance: 0, name: "Pedals".into() };
        let wheel = Numbered { vid: 0x046d, pid: 0xc24f, instance: 0, name: "Wheel".into() };
        let wheel2 = Numbered { instance: 1, ..wheel.clone() };
        let order = numbered(vec![wheel2.clone(), pedals.clone(), wheel.clone()]);
        assert_eq!(order, vec![wheel, wheel2, pedals]);
    }
}
