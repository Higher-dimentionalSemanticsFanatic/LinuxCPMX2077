//! Game controllers through hid.dll: list them and read their input
//! reports. The same calls work on Windows and inside Proton, where Wine's
//! own DirectInput reads joysticks this way.

use std::ptr::null_mut;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_BUTTON_CAPS, HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, HidD_FreePreparsedData,
    HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData, HidD_GetProductString, HidP_GetButtonCaps, HidP_GetCaps,
    HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps, HidP_Input, HidP_MaxUsageListLength, PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile};

use crate::{GD_HAT_SWITCH, Layout, Live, PAGE_BUTTON, PAGE_GENERIC, PAGE_SIMULATION, usage};

/// A connected game controller.
#[derive(Debug, Clone)]
pub struct Info {
    /// Device interface path, NUL-terminated.
    pub path: Vec<u16>,
    pub vid: u16,
    pub pid: u16,
    /// Which of several identical devices, by path order.
    pub instance: u32,
    pub name: String,
    pub layout: Layout,
    /// An XInput gamepad (`IG_` in its path): the game reads it already, and
    /// the in-game menu leaves it out of its controller numbers.
    pub xinput: bool,
}

#[derive(Debug, Clone, Copy)]
struct ValueCap {
    page: u16,
    usage: u16,
    link: u16,
    min: i32,
    max: i32,
    bits: u16,
    data_index: u16,
}

#[derive(Debug, Clone, Copy)]
struct ButtonCap {
    usage: u16,
    report_id: u8,
}

/// What a device declares, read from its preparsed data.
struct Caps {
    axes: Vec<ValueCap>,
    hats: Vec<ValueCap>,
    buttons: Vec<ButtonCap>,
    report_len: usize,
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct Preparsed(PHIDP_PREPARSED_DATA);

impl Drop for Preparsed {
    fn drop(&mut self) {
        unsafe { HidD_FreePreparsedData(self.0) };
    }
}

fn open(path: &[u16], access: u32) -> Option<Handle> {
    let h = unsafe {
        CreateFileW(path.as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE, null_mut(), OPEN_EXISTING, 0, null_mut())
    };
    (h != INVALID_HANDLE_VALUE && !h.is_null()).then_some(Handle(h))
}

fn preparsed(h: &Handle) -> Option<Preparsed> {
    let mut p: PHIDP_PREPARSED_DATA = 0;
    (unsafe { HidD_GetPreparsedData(h.0, &mut p) } && p != 0).then_some(Preparsed(p))
}

/// The device's capabilities, if it is a joystick, gamepad, multi-axis
/// controller or simulation device.
fn caps(p: &Preparsed) -> Option<Caps> {
    let mut c: HIDP_CAPS = unsafe { std::mem::zeroed() };
    if unsafe { HidP_GetCaps(p.0, &mut c) } != HIDP_STATUS_SUCCESS {
        return None;
    }
    let controller = (c.UsagePage == PAGE_GENERIC && matches!(c.Usage, 0x04 | 0x05 | 0x08)) || c.UsagePage == PAGE_SIMULATION;
    if !controller {
        return None;
    }
    let mut values = Vec::new();
    let mut n = c.NumberInputValueCaps;
    if n > 0 {
        let mut raw: Vec<HIDP_VALUE_CAPS> = vec![HIDP_VALUE_CAPS::default(); n as usize];
        if unsafe { HidP_GetValueCaps(HidP_Input, raw.as_mut_ptr(), &mut n, p.0) } == HIDP_STATUS_SUCCESS {
            for v in &raw[..n as usize] {
                let (lo, hi, idx) = if v.IsRange {
                    let r = unsafe { v.Anonymous.Range };
                    (r.UsageMin, r.UsageMax, r.DataIndexMin)
                } else {
                    let r = unsafe { v.Anonymous.NotRange };
                    (r.Usage, r.Usage, r.DataIndex)
                };
                for k in 0..=hi.saturating_sub(lo).min(64) {
                    values.push(ValueCap {
                        page: v.UsagePage,
                        usage: lo + k,
                        link: v.LinkCollection,
                        min: v.LogicalMin,
                        max: v.LogicalMax,
                        bits: v.BitSize,
                        data_index: idx.saturating_add(k),
                    });
                }
            }
        }
    }
    // Descriptor order.
    values.sort_by_key(|v| v.data_index);
    let (hats, axes): (Vec<_>, Vec<_>) = values
        .into_iter()
        .filter(|v| v.page == PAGE_GENERIC || v.page == PAGE_SIMULATION)
        .partition(|v| v.page == PAGE_GENERIC && v.usage == GD_HAT_SWITCH);
    let mut buttons = Vec::new();
    let mut n = c.NumberInputButtonCaps;
    if n > 0 {
        let mut raw: Vec<HIDP_BUTTON_CAPS> = vec![HIDP_BUTTON_CAPS::default(); n as usize];
        if unsafe { HidP_GetButtonCaps(HidP_Input, raw.as_mut_ptr(), &mut n, p.0) } == HIDP_STATUS_SUCCESS {
            for b in raw[..n as usize].iter().filter(|b| b.UsagePage == PAGE_BUTTON) {
                let (lo, hi) = if b.IsRange {
                    let r = unsafe { b.Anonymous.Range };
                    (r.UsageMin, r.UsageMax)
                } else {
                    let r = unsafe { b.Anonymous.NotRange };
                    (r.Usage, r.Usage)
                };
                for u in lo..=hi.min(lo.saturating_add(255)) {
                    buttons.push(ButtonCap { usage: u, report_id: b.ReportID });
                }
            }
        }
    }
    buttons.sort_by_key(|b| b.usage);
    buttons.dedup_by_key(|b| b.usage);
    Some(Caps { axes, hats, buttons, report_len: c.InputReportByteLength as usize })
}

fn layout(c: &Caps) -> Layout {
    Layout { axes: c.axes.iter().map(|a| usage(a.page, a.usage)).collect(), buttons: c.buttons.len(), hats: c.hats.len() }
}

fn product_name(h: &Handle) -> String {
    let mut buf = [0u16; 256];
    if !unsafe { HidD_GetProductString(h.0, buf.as_mut_ptr().cast(), (buf.len() * 2) as u32) } {
        return String::new();
    }
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().to_string()
}

/// HID device interface paths, NUL-terminated, sorted.
fn interface_paths() -> Vec<Vec<u16>> {
    let mut guid = unsafe { std::mem::zeroed() };
    unsafe { HidD_GetHidGuid(&mut guid) };
    let set = unsafe { SetupDiGetClassDevsW(&guid, std::ptr::null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) };
    if set == -1 || set == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0..256 {
        let mut iface: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
        iface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        if unsafe { SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &guid, index, &mut iface) } == 0 {
            break;
        }
        let mut needed = 0u32;
        unsafe { SetupDiGetDeviceInterfaceDetailW(set, &iface, null_mut(), 0, &mut needed, null_mut()) };
        if !(8..=4096).contains(&needed) {
            continue;
        }
        // u64 backing keeps the detail struct aligned.
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        if unsafe { SetupDiGetDeviceInterfaceDetailW(set, &iface, detail, needed, null_mut(), null_mut()) } == 0 {
            continue;
        }
        let start = unsafe { std::ptr::addr_of!((*detail).DevicePath).cast::<u16>() };
        let max = (needed as usize - 4) / 2;
        let chars = unsafe { std::slice::from_raw_parts(start, max) };
        let len = chars.iter().position(|c| *c == 0).unwrap_or(max);
        let mut path = chars[..len].to_vec();
        path.push(0);
        out.push(path);
    }
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    out.sort();
    out
}

/// Every connected game controller.
pub fn list() -> Vec<Info> {
    let mut out: Vec<Info> = Vec::new();
    for path in interface_paths() {
        // No access needed to read attributes and descriptors.
        let Some(h) = open(&path, 0) else { continue };
        let Some(p) = preparsed(&h) else { continue };
        let Some(c) = caps(&p) else { continue };
        let mut attr: HIDD_ATTRIBUTES = unsafe { std::mem::zeroed() };
        attr.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
        if !unsafe { HidD_GetAttributes(h.0, &mut attr) } {
            continue;
        }
        let instance = out.iter().filter(|i| i.vid == attr.VendorID && i.pid == attr.ProductID).count() as u32;
        let xinput = String::from_utf16_lossy(&path).to_ascii_lowercase().contains("ig_");
        out.push(Info {
            path,
            vid: attr.VendorID,
            pid: attr.ProductID,
            instance,
            name: product_name(&h),
            layout: layout(&c),
            xinput,
        });
    }
    out
}

/// Reads one device's input reports into a [`Live`].
pub struct Reader {
    handle: Handle,
    preparsed: Preparsed,
    caps: Caps,
    report: Vec<u8>,
    pressed: Vec<u16>,
}

// The handle and preparsed data are only used by the thread owning the reader.
unsafe impl Send for Reader {}

impl Reader {
    pub fn open(path: &[u16]) -> Option<Reader> {
        let handle = open(path, GENERIC_READ)?;
        let preparsed = preparsed(&handle)?;
        let caps = caps(&preparsed)?;
        let max = unsafe { HidP_MaxUsageListLength(HidP_Input, PAGE_BUTTON, preparsed.0) } as usize;
        let report = vec![0u8; caps.report_len.max(1)];
        Some(Reader { handle, preparsed, caps, report, pressed: vec![0; max.max(1)] })
    }

    pub fn layout(&self) -> Layout {
        layout(&self.caps)
    }

    /// A [`Live`] sized for this device, everything at rest.
    pub fn idle(&self) -> Live {
        Live {
            axes: vec![0.0; self.caps.axes.len()],
            buttons: vec![false; self.caps.buttons.len()],
            hats: vec![None; self.caps.hats.len()],
        }
    }

    /// Wait for the next input report and apply it to `live`. False when
    /// the device is gone.
    pub fn read(&mut self, live: &mut Live) -> bool {
        let mut got = 0u32;
        let ok = unsafe {
            ReadFile(self.handle.0, self.report.as_mut_ptr(), self.report.len() as u32, &mut got, null_mut())
        };
        if ok == 0 || got == 0 {
            return false;
        }
        self.apply(live, got as usize);
        true
    }

    fn value(&self, v: &ValueCap, len: usize) -> Option<i64> {
        let mut raw = 0u32;
        let st = unsafe {
            HidP_GetUsageValue(HidP_Input, v.page, v.link, v.usage, &mut raw, self.preparsed.0, self.report.as_ptr(), len as u32)
        };
        if st != HIDP_STATUS_SUCCESS {
            // Not in this report (another report ID): keep the last value.
            return None;
        }
        Some(if v.min < 0 && (1..32).contains(&v.bits) {
            let shift = 32 - v.bits as u32;
            (((raw << shift) as i32) >> shift) as i64
        } else {
            raw as i64
        })
    }

    fn apply(&mut self, live: &mut Live, len: usize) {
        for (i, a) in self.caps.axes.iter().enumerate() {
            let Some(v) = self.value(a, len) else { continue };
            let (min, mut max) = (a.min as i64, a.max as i64);
            // Some devices declare an unsigned 16-bit maximum as -1.
            if max <= min && min >= 0 && (1..32).contains(&a.bits) {
                max = (1i64 << a.bits) - 1;
            }
            if max > min {
                live.axes[i] = ((v - min) as f32 / (max - min) as f32).clamp(0.0, 1.0);
            }
        }
        for (i, h) in self.caps.hats.iter().enumerate() {
            let Some(v) = self.value(h, len) else { continue };
            let (min, max) = (h.min as i64, h.max as i64);
            live.hats[i] = (max > min && v >= min && v <= max).then(|| ((v - min) * 8 / (max - min + 1)) as u8);
        }
        if self.caps.buttons.is_empty() {
            return;
        }
        let mut n = self.pressed.len() as u32;
        let st = unsafe {
            HidP_GetUsages(HidP_Input, PAGE_BUTTON, 0, self.pressed.as_mut_ptr(), &mut n, self.preparsed.0, self.report.as_mut_ptr(), len as u32)
        };
        if st != HIDP_STATUS_SUCCESS {
            return;
        }
        let id = self.report[0];
        let pressed = &self.pressed[..n as usize];
        for (i, b) in self.caps.buttons.iter().enumerate() {
            // Buttons in other reports keep their state.
            if b.report_id == id {
                live.buttons[i] = pressed.contains(&b.usage);
            }
        }
    }
}
