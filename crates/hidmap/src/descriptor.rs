//! HID report descriptors: which axes, buttons and hats a device declares,
//! and the evdev axis Linux's generic HID driver turns each axis into. On
//! Linux CPMX2077 reads the descriptor from sysfs to learn the usage behind
//! each evdev axis, the usage Windows and Proton's hidraw path show the game.

use crate::{GD_HAT_SWITCH, PAGE_BUTTON, PAGE_GENERIC, PAGE_SIMULATION, usage};

/// The inputs a descriptor declares, in descriptor order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub axes: Vec<u32>,
    /// Button usage ids (Button page), as declared.
    pub buttons: Vec<u16>,
    pub hats: usize,
}

#[derive(Clone, Copy, Default)]
struct Globals {
    page: u16,
    logical_min: i32,
    logical_max: i32,
    report_size: u32,
    report_count: u32,
}

/// Parse the Input items of a report descriptor. Constant fields, arrays
/// (keyboard-style) and vendor pages are left out; a truncated descriptor
/// yields what was read before the cut.
pub fn parse(desc: &[u8]) -> Parsed {
    let mut out = Parsed::default();
    let mut g = Globals::default();
    let mut stack: Vec<Globals> = Vec::new();
    let mut usages: Vec<u32> = Vec::new();
    let (mut umin, mut umax): (Option<u32>, Option<u32>) = (None, None);
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: size byte, tag byte, data.
            let size = desc.get(i + 1).copied().unwrap_or(0) as usize;
            i += 3 + size;
            continue;
        }
        let size = match prefix & 3 {
            3 => 4,
            n => n as usize,
        };
        if i + 1 + size > desc.len() {
            break;
        }
        let data = &desc[i + 1..i + 1 + size];
        let unsigned = data.iter().rev().fold(0u32, |a, b| a << 8 | *b as u32);
        let signed = match size {
            1 => data[0] as i8 as i32,
            2 => i16::from_le_bytes([data[0], data[1]]) as i32,
            4 => unsigned as i32,
            _ => 0,
        };
        let full_usage = |v: u32, page: u16| if size == 4 { v } else { usage(page, v as u16) };
        match (prefix >> 2) & 3 {
            // Main.
            0 => {
                let tag = prefix >> 4;
                if tag == 0x8 {
                    input(&mut out, g, unsigned, &usages, umin, umax);
                }
                // Local items end with every main item.
                usages.clear();
                umin = None;
                umax = None;
            }
            // Global.
            1 => match prefix >> 4 {
                0x0 => g.page = unsigned as u16,
                0x1 => g.logical_min = signed,
                0x2 => g.logical_max = signed,
                0x7 => g.report_size = unsigned,
                0x9 => g.report_count = unsigned,
                0xA => stack.push(g),
                0xB => g = stack.pop().unwrap_or(g),
                _ => {}
            },
            // Local.
            2 => match prefix >> 4 {
                0x0 => usages.push(full_usage(unsigned, g.page)),
                0x1 => umin = Some(full_usage(unsigned, g.page)),
                0x2 => umax = Some(full_usage(unsigned, g.page)),
                _ => {}
            },
            _ => {}
        }
        i += 1 + size;
    }
    out
}

fn input(out: &mut Parsed, g: Globals, flags: u32, usages: &[u32], umin: Option<u32>, umax: Option<u32>) {
    let constant = flags & 1 != 0;
    let variable = flags & 2 != 0;
    if constant || !variable || g.report_count == 0 {
        return;
    }
    // Explicit usages first, then the range; the last one repeats.
    let mut list: Vec<u32> = usages.to_vec();
    if let (Some(a), Some(b)) = (umin, umax)
        && b >= a
        && b - a < 4096
    {
        list.extend(a..=b);
    }
    if list.is_empty() {
        return;
    }
    for n in 0..g.report_count.min(4096) as usize {
        let u = *list.get(n).unwrap_or(list.last().unwrap_or(&0));
        let (page, id) = ((u >> 16) as u16, u as u16);
        if page >= 0xFF00 {
            continue;
        }
        if page == PAGE_BUTTON {
            out.buttons.push(id);
        } else if page == PAGE_GENERIC && id == GD_HAT_SWITCH {
            out.hats += 1;
        } else if g.report_size > 1 && (page == PAGE_GENERIC || page == PAGE_SIMULATION) {
            out.axes.push(u);
        }
    }
}

/// The evdev `ABS_*` code Linux's generic HID input driver gives an
/// absolute axis usage (`hid-input.c`), or none when it ignores it.
pub fn evdev_abs(u: u32) -> Option<u16> {
    let (page, id) = ((u >> 16) as u16, u as u16);
    match page {
        // X..Rz map to ABS_X..ABS_RZ, Slider, Dial and Wheel to 6, 7 and 8
        // (ABS_THROTTLE, ABS_RUDDER, ABS_WHEEL): the usage's low nibble.
        PAGE_GENERIC if (0x30..=0x38).contains(&id) => Some(id & 0xF),
        PAGE_SIMULATION => match id {
            0xBA => Some(0x07), // ABS_RUDDER
            0xBB => Some(0x06), // ABS_THROTTLE
            0xC4 => Some(0x09), // ABS_GAS
            0xC5 => Some(0x0A), // ABS_BRAKE
            0xC8 => Some(0x08), // ABS_WHEEL
            _ => None,
        },
        _ => None,
    }
}

/// For each evdev axis code, the descriptor axis behind it: its usage and
/// which occurrence of that usage it is. The driver drops a usage whose
/// code is already taken, so only the first axis with a code gets it.
pub fn usage_of_code(axes: &[u32], code: u16) -> Option<(u32, u32)> {
    let mut taken = [false; 0x40];
    for (i, u) in axes.iter().enumerate() {
        let Some(c) = evdev_abs(*u) else { continue };
        if taken[c as usize] {
            continue;
        }
        taken[c as usize] = true;
        if c == code {
            let occurrence = axes[..i].iter().filter(|a| *a == u).count() as u32;
            return Some((*u, occurrence));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pedals with accelerator and brake (Simulation page), an X axis, four
    /// buttons, padding and a hat switch.
    const PEDALS: &[u8] = &[
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x04, // Usage (Joystick)
        0xA1, 0x01, // Collection (Application)
        0x05, 0x02, //   Usage Page (Simulation)
        0x09, 0xC4, //   Usage (Accelerator)
        0x09, 0xC5, //   Usage (Brake)
        0x15, 0x00, //   Logical Minimum (0)
        0x26, 0xFF, 0x03, // Logical Maximum (1023)
        0x75, 0x10, //   Report Size (16)
        0x95, 0x02, //   Report Count (2)
        0x81, 0x02, //   Input (Data, Variable, Absolute)
        0x05, 0x01, //   Usage Page (Generic Desktop)
        0x09, 0x30, //   Usage (X)
        0x26, 0xFF, 0x00, // Logical Maximum (255)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x02, //   Input
        0x05, 0x09, //   Usage Page (Button)
        0x19, 0x01, //   Usage Minimum (1)
        0x29, 0x04, //   Usage Maximum (4)
        0x25, 0x01, //   Logical Maximum (1)
        0x75, 0x01, //   Report Size (1)
        0x95, 0x04, //   Report Count (4)
        0x81, 0x02, //   Input
        0x81, 0x01, //   Input (Constant): padding
        0x05, 0x01, //   Usage Page (Generic Desktop)
        0x09, 0x39, //   Usage (Hat switch)
        0x25, 0x07, //   Logical Maximum (7)
        0x75, 0x04, //   Report Size (4)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x42, //   Input (Data, Variable, Absolute, Null state)
        0x06, 0x00, 0xFF, // Usage Page (Vendor)
        0x09, 0x01, //   Usage (1)
        0x75, 0x08, 0x95, 0x02, 0x81, 0x02, // vendor bytes
        0xC0, // End Collection
    ];

    #[test]
    fn parses_axes_buttons_and_hats() {
        let p = parse(PEDALS);
        assert_eq!(p.axes, vec![usage(2, 0xC4), usage(2, 0xC5), usage(1, 0x30)]);
        assert_eq!(p.buttons, vec![1, 2, 3, 4]);
        assert_eq!(p.hats, 1);
        // A cut-off descriptor keeps what came before the cut.
        assert_eq!(parse(&PEDALS[..24]).axes.len(), 2);
        assert_eq!(parse(&[]), Parsed::default());
    }

    #[test]
    fn maps_axes_to_evdev_codes() {
        let p = parse(PEDALS);
        assert_eq!(usage_of_code(&p.axes, 0x00), Some((usage(1, 0x30), 0)), "X is ABS_X");
        assert_eq!(usage_of_code(&p.axes, 0x09), Some((usage(2, 0xC4), 0)), "Accelerator is ABS_GAS");
        assert_eq!(usage_of_code(&p.axes, 0x0A), Some((usage(2, 0xC5), 0)), "Brake is ABS_BRAKE");
        assert_eq!(usage_of_code(&p.axes, 0x05), None);
        assert_eq!(evdev_abs(usage(1, 0x36)), Some(6), "Slider is ABS_THROTTLE");
        // A second Z axis is dropped by the driver.
        let z = usage(1, 0x32);
        assert_eq!(usage_of_code(&[z, z], 0x02), Some((z, 0)));
    }
}
