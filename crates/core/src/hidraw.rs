//! Game controllers the Linux kernel exposes as hidraw nodes, for
//! CyberHID's Proton setup (see [`crate::linux_setup`]).
//!
//! Proton hands a joystick to the game either through SDL, as a device with
//! a generated HID descriptor (at most 6 axes per device, generic axis
//! names, and one with 6 axes and 14+ buttons turned into an XInput pad), or
//! as the raw device with its own descriptor. The raw path is used for the
//! VID/PID pairs `PROTON_ENABLE_HIDRAW` names (winebus in Proton 9 and 10)
//! and needs read/write access to the `/dev/hidrawN` node; without access
//! Proton stays on SDL.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The launch option that makes Proton pass controllers as raw HID devices.
pub const PROTON_VAR: &str = "PROTON_ENABLE_HIDRAW";

/// A joystick-class HID device: wheel, pedals, stick, throttle, button box.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Controller {
    pub vid: u16,
    pub pid: u16,
    pub name: String,
    /// Its `/dev/hidrawN` nodes (one per controller interface).
    pub nodes: Vec<PathBuf>,
    /// Every node opens for reading and writing, as Wine opens them.
    pub access: bool,
}

impl Controller {
    /// `0x046d/0xc24f`, the form `PROTON_ENABLE_HIDRAW` matches (any case).
    pub fn proton_id(&self) -> String {
        format!("0x{:04x}/0x{:04x}", self.vid, self.pid)
    }

    /// `Logitech G29 (046d:c24f)`.
    pub fn label(&self) -> String {
        format!("{} ({:04x}:{:04x})", self.name, self.vid, self.pid)
    }
}

/// `(usage page, usage)` of each top-level application collection in a HID
/// report descriptor.
pub fn applications(desc: &[u8]) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    let mut page = 0u16;
    let mut pushed = Vec::new();
    // Usages since the last main item; a 4-byte usage carries its own page.
    let mut usages: Vec<(Option<u16>, u16)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: size, tag, data.
            let Some(size) = desc.get(i + 1) else { break };
            i += 3 + *size as usize;
            continue;
        }
        let size = match prefix & 3 {
            3 => 4,
            n => n as usize,
        };
        let Some(data) = desc.get(i + 1..i + 1 + size) else { break };
        let value = data.iter().rev().fold(0u32, |a, b| (a << 8) | *b as u32);
        i += 1 + size;
        match prefix & 0xFC {
            0x04 => page = value as u16,
            0xA4 => pushed.push(page),
            0xB4 => page = pushed.pop().unwrap_or(page),
            0x08 if size == 4 => usages.push((Some((value >> 16) as u16), value as u16)),
            0x08 => usages.push((None, value as u16)),
            0xA0 => {
                if depth == 0
                    && value == 1
                    && let Some((p, u)) = usages.first()
                {
                    out.push((p.unwrap_or(page), *u));
                }
                depth += 1;
                usages.clear();
            }
            0xC0 => {
                depth = depth.saturating_sub(1);
                usages.clear();
            }
            0x80 | 0x90 | 0xB0 => usages.clear(),
            _ => {}
        }
    }
    out
}

/// A joystick (Generic Desktop 0x04), multi-axis controller (0x08) or
/// simulation device (page 0x02: wheels, pedals, throttles). Gamepads (0x05)
/// are left out: Proton gives them to the game as XInput pads, which the
/// game reads itself.
pub fn is_controller(apps: &[(u16, u16)]) -> bool {
    apps.iter().any(|&(page, usage)| (page == 0x01 && (usage == 0x04 || usage == 0x08)) || page == 0x02)
}

/// VID, PID and name from a HID device's `uevent`
/// (`HID_ID=0003:0000046D:0000C24F`, `HID_NAME=...`).
fn uevent(s: &str) -> Option<(u16, u16, String)> {
    let get = |k: &str| s.lines().find_map(|l| l.strip_prefix(k)).map(str::trim);
    let mut id = get("HID_ID=")?.split(':').skip(1);
    let vid = u16::try_from(u32::from_str_radix(id.next()?, 16).ok()?).ok()?;
    let pid = u16::try_from(u32::from_str_radix(id.next()?, 16).ok()?).ok()?;
    let name: String = get("HID_NAME=").unwrap_or_default().chars().filter(|c| !c.is_control()).take(120).collect();
    let name = if name.trim().is_empty() { "HID controller".to_string() } else { name.trim().to_string() };
    Some((vid, pid, name))
}

/// Controllers listed under `sys` (`/sys/class/hidraw`), with their nodes
/// under `dev` (`/dev`). Devices with the same VID/PID are one entry.
pub fn scan_in(sys: &Path, dev: &Path) -> Vec<Controller> {
    let Ok(rd) = std::fs::read_dir(sys) else { return Vec::new() };
    let mut names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("hidraw"))
        .collect();
    names.sort_by_key(|n| n["hidraw".len()..].parse::<u32>().unwrap_or(u32::MAX));
    let mut out: Vec<Controller> = Vec::new();
    for n in names {
        let device = sys.join(&n).join("device");
        let Ok(desc) = std::fs::read(device.join("report_descriptor")) else { continue };
        if !is_controller(&applications(&desc)) {
            continue;
        }
        let Some((vid, pid, name)) = std::fs::read_to_string(device.join("uevent")).ok().and_then(|s| uevent(&s)) else { continue };
        let node = dev.join(&n);
        let access = std::fs::OpenOptions::new().read(true).write(true).open(&node).is_ok();
        match out.iter_mut().find(|c| c.vid == vid && c.pid == pid) {
            Some(c) => {
                c.nodes.push(node);
                c.access &= access;
            }
            None => out.push(Controller { vid, pid, name, nodes: vec![node], access }),
        }
    }
    out
}

/// The game controllers connected to this machine (none off Linux).
pub fn scan() -> Vec<Controller> {
    if cfg!(target_os = "linux") { scan_in(Path::new("/sys/class/hidraw"), Path::new("/dev")) } else { Vec::new() }
}

/// `PROTON_ENABLE_HIDRAW` already covers `c`: its VID/PID is listed, or the
/// value is `1` (every device, Proton 10).
pub fn listed(value: &str, c: &Controller) -> bool {
    let v = value.trim();
    v == "1" || v.to_ascii_lowercase().contains(&c.proton_id())
}

/// `value` with the controllers it doesn't list yet added (comma-separated).
pub fn with_listed(value: Option<&str>, add: &[&Controller]) -> String {
    let mut parts: Vec<String> =
        value.unwrap_or_default().split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    for c in add {
        if !parts.iter().any(|p| p.eq_ignore_ascii_case(&c.proton_id())) {
            parts.push(c.proton_id());
        }
    }
    parts.join(",")
}

/// Shell command that installs a udev rule giving the logged-in user access
/// to these controllers' hidraw nodes, then applies it.
pub fn udev_command(list: &[&Controller]) -> String {
    let rules: Vec<String> = list
        .iter()
        .map(|c| format!("'SUBSYSTEM==\"hidraw\", KERNELS==\"*:{:04X}:{:04X}.*\", TAG+=\"uaccess\"'", c.vid, c.pid))
        .collect();
    format!(
        "printf '%s\\n' {} | sudo tee /etc/udev/rules.d/70-cyberhid.rules >/dev/null && sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=hidraw",
        rules.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Logitech G29-style wheel: Generic Desktop / Joystick application.
    const JOYSTICK: &[u8] = &[
        0x05, 0x01, 0x09, 0x04, 0xA1, 0x01, 0xA1, 0x02, 0x09, 0x30, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x01, 0x81,
        0x02, 0xC0, 0xC0,
    ];
    // Xbox-style gamepad.
    const GAMEPAD: &[u8] = &[0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x09, 0x30, 0x81, 0x02, 0xC0];
    // Keyboard plus a consumer-control collection.
    const KEYBOARD: &[u8] = &[0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0xC0, 0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0xC0];

    #[test]
    fn reads_top_level_collections() {
        assert_eq!(applications(JOYSTICK), [(0x01, 0x04)], "the nested physical collection is not an application");
        assert_eq!(applications(KEYBOARD), [(0x01, 0x06), (0x0C, 0x01)]);
        // Simulation Controls / Automobile, usage page given after the usage
        // through a 4-byte usage; then Push/Pop around a vendor page.
        let pedals = [0x0B, 0x02, 0x00, 0x02, 0x00, 0xA1, 0x01, 0xA4, 0x06, 0x00, 0xFF, 0xB4, 0xC0];
        assert_eq!(applications(&pedals), [(0x02, 0x02)]);
        // A long item and a truncated tail don't derail it.
        let odd = [0xFE, 0x01, 0x10, 0x00, 0x05, 0x01, 0x09, 0x08, 0xA1, 0x01, 0xC0, 0x26, 0xFF];
        assert_eq!(applications(&odd), [(0x01, 0x08)]);
        assert!(is_controller(&applications(JOYSTICK)));
        assert!(is_controller(&applications(&pedals)));
        assert!(!is_controller(&applications(GAMEPAD)));
        assert!(!is_controller(&applications(KEYBOARD)));
    }

    #[test]
    fn scans_sysfs_for_controllers() {
        let tmp = tempfile::tempdir().unwrap();
        let (sys, dev) = (tmp.path().join("sys"), tmp.path().join("dev"));
        let add = |n: &str, desc: &[u8], id: &str, name: &str| {
            let d = sys.join(n).join("device");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("report_descriptor"), desc).unwrap();
            std::fs::write(d.join("uevent"), format!("DRIVER=hid-generic\nHID_ID={id}\nHID_NAME={name}\nHID_PHYS=usb-1\n")).unwrap();
        };
        std::fs::create_dir_all(&dev).unwrap();
        add("hidraw10", JOYSTICK, "0003:0000044F:0000B679", "Thrustmaster T-Rudder");
        add("hidraw2", JOYSTICK, "0003:0000046D:0000C24F", "Logitech G29 Driving Force Racing Wheel");
        add("hidraw3", JOYSTICK, "0003:0000046D:0000C24F", "Logitech G29 Driving Force Racing Wheel");
        add("hidraw4", GAMEPAD, "0003:0000045E:00000B12", "Microsoft Xbox Controller");
        add("hidraw5", KEYBOARD, "0003:000004D9:00001702", "Keyboard");
        std::fs::write(dev.join("hidraw2"), b"").unwrap();
        std::fs::write(dev.join("hidraw3"), b"").unwrap();

        let found = scan_in(&sys, &dev);
        assert_eq!(found.len(), 2, "{found:?}");
        let g29 = &found[0];
        assert_eq!((g29.vid, g29.pid), (0x046d, 0xc24f), "hidraw2 sorts before hidraw10");
        assert_eq!(g29.nodes, [dev.join("hidraw2"), dev.join("hidraw3")]);
        assert!(g29.access);
        assert_eq!(g29.label(), "Logitech G29 Driving Force Racing Wheel (046d:c24f)");
        assert_eq!(found[1].proton_id(), "0x044f/0xb679");
        assert!(!found[1].access, "no node to open");
        assert!(scan_in(&tmp.path().join("missing"), &dev).is_empty());
    }

    #[test]
    fn builds_the_launch_value_and_udev_rule() {
        let c = |vid, pid| Controller { vid, pid, name: "x".into(), nodes: vec![], access: false };
        let (wheel, rudder) = (c(0x046d, 0xc24f), c(0x044f, 0xb679));
        assert!(listed("0x046D/0xC24F", &wheel), "any case");
        assert!(listed("1", &rudder));
        assert!(!listed("0x046d/0xc24f", &rudder));
        assert_eq!(with_listed(None, &[&wheel, &rudder]), "0x046d/0xc24f,0x044f/0xb679");
        assert_eq!(with_listed(Some("0x3344/0x0001, 0x046D/0xC24F"), &[&wheel, &rudder]), "0x3344/0x0001,0x046D/0xC24F,0x044f/0xb679");
        assert_eq!(
            udev_command(&[&wheel]),
            "printf '%s\\n' 'SUBSYSTEM==\"hidraw\", KERNELS==\"*:046D:C24F.*\", TAG+=\"uaccess\"' | sudo tee /etc/udev/rules.d/70-cyberhid.rules >/dev/null && sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=hidraw"
        );
    }
}
