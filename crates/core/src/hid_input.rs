//! HID input page: CPMX2077 HID Input, a RED4ext plugin built from this
//! workspace (`crates/hid-plugin`) that adds pedals, wheels, flight sticks and
//! button boxes to the gamepad the game reads (`cp2077mm_hidmap` has the
//! bindings format and the merge rules). This module lists the game
//! controllers connected to this computer and reads them live for binding,
//! reads and writes the plugin's bindings file, reads the status file the
//! plugin writes while the game runs, and installs the plugin as a tracked
//! mod.
//!
//! On Linux devices are read through evdev and sysfs, the same devices
//! Proton passes to the game; the HID descriptor in sysfs gives each evdev
//! axis the usage Windows and Proton's hidraw path show. On Windows they are
//! read through hid.dll, as the plugin does.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use cp2077mm_hidmap::{self as hm, Bindings, Input, PAGE_SIMULATION, Status, menu, usage};
use serde::Serialize;

use crate::db::{Db, GameRow, ModRow, NewMod};
use crate::install::{InstallOptions, InstallReport, Installer, resolve_ci};
use crate::{Error, Result, hash};

/// `source` of the plugin's mod record.
pub const SOURCE: &str = "cpmx-hid";
pub const MOD_NAME: &str = hm::PLUGIN_NAME;

// ---- devices on this computer --------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AxisView {
    /// Position among the device's axes ([`Input::Axis`] `index`).
    pub index: u32,
    /// The usage in the device's own HID descriptor, when read.
    pub usage: Option<u32>,
    pub occurrence: u32,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceView {
    pub id: String,
    pub vid: u16,
    pub pid: u16,
    pub instance: u32,
    pub name: String,
    /// Picture to draw: pedals, wheel, stick, throttle, buttonbox, gamepad
    /// or generic.
    pub kind: &'static str,
    /// Axis usages from the device's HID descriptor, empty when unread.
    pub hid_axes: Vec<u32>,
    pub axes: Vec<AxisView>,
    pub buttons: usize,
    pub hats: usize,
    /// Why live values are missing, if they are.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct LiveView {
    pub id: String,
    pub axes: Vec<f32>,
    pub buttons: Vec<bool>,
    pub hats: Vec<Option<u8>>,
}

/// The picture for a device: from its name, else from its inputs.
pub fn guess_kind(name: &str, axes: &[u32], buttons: usize, hats: usize) -> &'static str {
    let n = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["pedal", "rudder", "tpr", "tfrp", "heusinkveld"]) {
        return "pedals";
    }
    if has(&["wheel", "g29", "g920", "g923", "g27", "g25", "driving force", "t300", "t150", "t248", "tmx", "fanatec", "moza", "simucube", "csl dd"]) {
        return "wheel";
    }
    if has(&["throttle", "twcs", "tqs"]) {
        return "throttle";
    }
    if has(&["stick", "hotas", "x52", "x56", "t.16000", "t16000", "vkb", "virpil", "gladiator", "warthog", "extreme 3d", "flight"]) {
        return "stick";
    }
    if has(&["button box", "buttonbox", "button-box"]) {
        return "buttonbox";
    }
    if has(&["xbox", "gamepad", "controller", "dualsense", "dualshock", "joy-con"]) {
        return "gamepad";
    }
    let sim = |id| axes.contains(&usage(PAGE_SIMULATION, id));
    if sim(0xC4) || sim(0xC5) || sim(0xBA) {
        return "pedals";
    }
    if sim(0xC8) {
        return "wheel";
    }
    if sim(0xBB) {
        return "throttle";
    }
    if buttons == 0 && hats == 0 && (1..=4).contains(&axes.len()) {
        return "pedals";
    }
    if axes.is_empty() && buttons > 0 {
        return "buttonbox";
    }
    if axes.len() >= 2 && hats > 0 {
        return "stick";
    }
    "generic"
}

fn axis_label(index: usize, usage: Option<u32>) -> String {
    match usage.and_then(hm::usage_label) {
        Some(l) => l.to_string(),
        None => format!("Axis {}", index + 1),
    }
}

/// Reads the game controllers connected to this computer. Kept between
/// calls so open devices stay open while the HID input page polls.
#[derive(Default)]
pub struct Scanner {
    #[cfg(target_os = "linux")]
    devs: Vec<evdev::Dev>,
    #[cfg(windows)]
    win: windows_side::Monitor,
}

impl Scanner {
    /// List the connected game controllers again.
    pub fn devices(&mut self) -> Vec<DeviceView> {
        #[cfg(target_os = "linux")]
        {
            self.devs = evdev::scan(Path::new("/sys/class/input"), Path::new("/run/udev/data"), Path::new("/dev/input"));
            self.devs.iter().map(|d| d.view.clone()).collect()
        }
        #[cfg(windows)]
        {
            self.win.devices()
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Vec::new()
        }
    }

    /// Current values of the listed devices.
    pub fn poll(&mut self) -> Vec<LiveView> {
        #[cfg(target_os = "linux")]
        {
            self.devs.iter_mut().filter_map(|d| d.poll()).collect()
        }
        #[cfg(windows)]
        {
            self.win.poll()
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Vec::new()
        }
    }
}

#[cfg(target_os = "linux")]
mod evdev {
    //! evdev nodes and their sysfs entries.

    use std::fs::File;
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};

    use cp2077mm_hidmap::{self as hm, PAGE_GENERIC, PAGE_SIMULATION, descriptor, usage};

    use super::{AxisView, DeviceView, LiveView, axis_label, guess_kind};

    const ABS_HAT0X: u16 = 0x10;
    const ABS_HAT3Y: u16 = 0x17;
    const ABS_MAX: u16 = 0x3f;
    const BTN_JOYSTICK: u16 = 0x120;
    const KEY_MAX: u16 = 0x2ff;

    pub struct Dev {
        pub view: DeviceView,
        node: PathBuf,
        /// Axis codes in position order (code order, hats left out).
        axes: Vec<u16>,
        /// The X code of each hat.
        hats: Vec<u16>,
        /// Button codes in position order.
        keys: Vec<u16>,
        file: Option<File>,
        tried: bool,
    }

    /// Set bits of a sysfs bitmap: hex words, most significant first.
    pub(super) fn bits(text: &str) -> Vec<u16> {
        let mut out = Vec::new();
        for (i, w) in text.split_whitespace().rev().enumerate() {
            let w = u64::from_str_radix(w, 16).unwrap_or(0);
            for b in 0..64 {
                if w >> b & 1 == 1 {
                    out.push((i * 64 + b) as u16);
                }
            }
        }
        out
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
    }

    /// The usage an evdev axis code stands for, when no descriptor is read.
    fn usage_of(code: u16) -> Option<u32> {
        match code {
            0..=5 => Some(usage(PAGE_GENERIC, 0x30 + code)),
            6 => Some(usage(PAGE_SIMULATION, 0xBB)),
            7 => Some(usage(PAGE_SIMULATION, 0xBA)),
            8 => Some(usage(PAGE_SIMULATION, 0xC8)),
            9 => Some(usage(PAGE_SIMULATION, 0xC4)),
            10 => Some(usage(PAGE_SIMULATION, 0xC5)),
            _ => None,
        }
    }

    /// Whether udev (or, without its database, the capabilities) marks the
    /// node as a joystick.
    fn is_joystick(dev: &Path, minor: Option<&str>, udev_data: &Path, abs: &[u16], keys: &[u16]) -> bool {
        if let Some(m) = minor
            && let Ok(db) = std::fs::read_to_string(udev_data.join(format!("c13:{m}")))
        {
            return db.lines().any(|l| l == "E:ID_INPUT_JOYSTICK=1");
        }
        let props = bits(&read(&dev.join("properties")));
        // Touchpads, pens and motion sensors also report absolute axes.
        let touch = keys.iter().any(|k| matches!(k, 0x140 | 0x145 | 0x14a));
        let accel = props.contains(&6);
        let axes = abs.iter().any(|a| *a <= 10);
        let pad_keys = keys.is_empty() || keys.iter().any(|k| (BTN_JOYSTICK..0x140).contains(k) || *k >= 0x2c0);
        axes && !touch && !accel && pad_keys
    }

    pub fn scan(sys: &Path, udev_data: &Path, dev_root: &Path) -> Vec<Dev> {
        let Ok(rd) = std::fs::read_dir(sys) else { return Vec::new() };
        let mut nodes: Vec<(u32, String)> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_prefix("event").and_then(|n| n.parse().ok()).map(|n| (n, name))
            })
            .collect();
        nodes.sort();
        let mut out: Vec<Dev> = Vec::new();
        for (_, event) in nodes {
            let dev = sys.join(&event).join("device");
            let abs = bits(&read(&dev.join("capabilities/abs")));
            let keys_all = bits(&read(&dev.join("capabilities/key")));
            let minor = read(&sys.join(&event).join("dev"));
            let minor = minor.split(':').nth(1);
            if !is_joystick(&dev, minor, udev_data, &abs, &keys_all) {
                continue;
            }
            let hex = |f: &str| u16::from_str_radix(&read(&dev.join("id").join(f)), 16).unwrap_or(0);
            let (vid, pid) = (hex("vendor"), hex("product"));
            let name = read(&dev.join("name"));
            let axes: Vec<u16> = abs.iter().copied().filter(|c| *c < ABS_MAX && !(ABS_HAT0X..=ABS_HAT3Y).contains(c)).collect();
            let hats: Vec<u16> =
                (ABS_HAT0X..=ABS_HAT3Y).step_by(2).filter(|h| abs.contains(h) || abs.contains(&(h + 1))).collect();
            // SDL's order, which Proton's SDL path keeps: joystick buttons and
            // up first, then the codes below.
            let mut keys: Vec<u16> = keys_all.iter().copied().filter(|k| (BTN_JOYSTICK..KEY_MAX).contains(k)).collect();
            keys.extend(keys_all.iter().copied().filter(|k| *k < BTN_JOYSTICK));
            // The HID descriptor of the device behind the node, when it is one.
            let parsed = std::fs::read(dev.join("device/report_descriptor")).ok().map(|d| descriptor::parse(&d));
            let hid_axes = parsed.map(|p| p.axes).unwrap_or_default();
            let views: Vec<AxisView> = axes
                .iter()
                .enumerate()
                .map(|(i, code)| {
                    let found = descriptor::usage_of_code(&hid_axes, *code);
                    let shown = found.map(|(u, _)| u).or_else(|| usage_of(*code));
                    AxisView {
                        index: i as u32,
                        usage: found.map(|(u, _)| u),
                        occurrence: found.map_or(0, |(_, o)| o),
                        label: axis_label(i, shown),
                    }
                })
                .collect();
            let shown: Vec<u32> = axes.iter().filter_map(|c| usage_of(*c)).collect();
            let kind_axes = if hid_axes.is_empty() { &shown } else { &hid_axes };
            let instance = out.iter().filter(|d| d.view.vid == vid && d.view.pid == pid).count() as u32;
            out.push(Dev {
                view: DeviceView {
                    id: hm::device_id(vid, pid, instance),
                    vid,
                    pid,
                    instance,
                    kind: guess_kind(&name, kind_axes, keys.len(), hats.len()),
                    name,
                    hid_axes,
                    axes: views,
                    buttons: keys.len(),
                    hats: hats.len(),
                    note: None,
                },
                node: dev_root.join(&event),
                axes,
                hats,
                keys,
                file: None,
                tried: false,
            });
        }
        out
    }

    #[repr(C)]
    #[derive(Default)]
    struct AbsInfo {
        value: i32,
        minimum: i32,
        maximum: i32,
        fuzz: i32,
        flat: i32,
        resolution: i32,
    }

    /// `_IOC(_IOC_READ, 'E', nr, size)`.
    const fn ioc_read(nr: u64, size: u64) -> u64 {
        (2 << 30) | (size << 16) | (0x45 << 8) | nr
    }

    fn abs(fd: i32, code: u16) -> Option<AbsInfo> {
        let mut info = AbsInfo::default();
        let req = ioc_read(0x40 + code as u64, size_of::<AbsInfo>() as u64);
        (unsafe { libc::ioctl(fd, req as _, &mut info as *mut AbsInfo) } >= 0).then_some(info)
    }

    impl Dev {
        pub fn poll(&mut self) -> Option<LiveView> {
            if self.file.is_none() && !self.tried {
                self.tried = true;
                match File::open(&self.node) {
                    Ok(f) => self.file = Some(f),
                    Err(e) => self.view.note = Some(format!("{}: {e}", self.node.display())),
                }
            }
            let fd = self.file.as_ref()?.as_raw_fd();
            let axes = self
                .axes
                .iter()
                .map(|c| match abs(fd, *c) {
                    Some(a) if a.maximum > a.minimum => {
                        ((a.value - a.minimum) as f32 / (a.maximum - a.minimum) as f32).clamp(0.0, 1.0)
                    }
                    _ => 0.0,
                })
                .collect();
            let hats = self
                .hats
                .iter()
                .map(|h| {
                    let x = abs(fd, *h).map_or(0, |a| a.value.signum());
                    let y = abs(fd, h + 1).map_or(0, |a| a.value.signum());
                    match (x, y) {
                        (0, -1) => Some(0),
                        (1, -1) => Some(1),
                        (1, 0) => Some(2),
                        (1, 1) => Some(3),
                        (0, 1) => Some(4),
                        (-1, 1) => Some(5),
                        (-1, 0) => Some(6),
                        (-1, -1) => Some(7),
                        _ => None,
                    }
                })
                .collect();
            let mut state = [0u8; (KEY_MAX as usize + 1) / 8];
            let req = ioc_read(0x18, state.len() as u64);
            let ok = unsafe { libc::ioctl(fd, req as _, state.as_mut_ptr()) } >= 0;
            let buttons = self
                .keys
                .iter()
                .map(|k| ok && state[*k as usize / 8] >> (k % 8) & 1 == 1)
                .collect();
            Some(LiveView { id: self.view.id.clone(), axes, buttons, hats })
        }
    }
}

#[cfg(windows)]
mod windows_side {
    //! hid.dll readers, one thread per device, as in the plugin.

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use cp2077mm_hidmap::{self as hm, Live, win};

    use super::{AxisView, DeviceView, LiveView, axis_label, guess_kind};

    #[derive(Default)]
    pub struct Monitor {
        listed: Vec<win::Info>,
        live: HashMap<Vec<u16>, Arc<Mutex<Option<Live>>>>,
    }

    impl Monitor {
        pub fn devices(&mut self) -> Vec<DeviceView> {
            self.listed = win::list();
            for info in &self.listed {
                let open = self.live.get(&info.path).is_some_and(|l| l.lock().unwrap().is_some());
                if open {
                    continue;
                }
                let Some(mut reader) = win::Reader::open(&info.path) else { continue };
                let slot = Arc::new(Mutex::new(Some(reader.idle())));
                self.live.insert(info.path.clone(), slot.clone());
                std::thread::spawn(move || {
                    let mut live = reader.idle();
                    while reader.read(&mut live) {
                        *slot.lock().unwrap() = Some(live.clone());
                    }
                    *slot.lock().unwrap() = None;
                });
            }
            self.listed
                .iter()
                .map(|i| DeviceView {
                    id: hm::device_id(i.vid, i.pid, i.instance),
                    vid: i.vid,
                    pid: i.pid,
                    instance: i.instance,
                    name: i.name.clone(),
                    kind: guess_kind(&i.name, &i.layout.axes, i.layout.buttons, i.layout.hats),
                    hid_axes: i.layout.axes.clone(),
                    axes: i
                        .layout
                        .axes
                        .iter()
                        .enumerate()
                        .map(|(n, u)| AxisView {
                            index: n as u32,
                            usage: Some(*u),
                            occurrence: i.layout.axes[..n].iter().filter(|a| *a == u).count() as u32,
                            label: axis_label(n, Some(*u)),
                        })
                        .collect(),
                    buttons: i.layout.buttons,
                    hats: i.layout.hats,
                    note: (!self.live.contains_key(&i.path)).then(|| "could not be opened for reading".to_string()),
                })
                .collect()
        }

        pub fn poll(&mut self) -> Vec<LiveView> {
            self.listed
                .iter()
                .filter_map(|i| {
                    let live = self.live.get(&i.path)?.lock().unwrap().clone()?;
                    Some(LiveView { id: hm::device_id(i.vid, i.pid, i.instance), axes: live.axes, buttons: live.buttons, hats: live.hats })
                })
                .collect()
        }
    }
}

// ---- bindings file ---------------------------------------------------------------

pub fn plugin_dir(game_dir: &Path) -> PathBuf {
    resolve_ci(game_dir, hm::PLUGIN_DIR)
}

/// The game's bindings, or empty ones before the first save.
pub fn load_bindings(game_dir: &Path) -> Result<Bindings> {
    let path = plugin_dir(game_dir).join(hm::BINDINGS_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| Error::Other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Bindings::default()),
        Err(e) => Err(e.into()),
    }
}

/// Check and write the bindings. The plugin picks the file up within a
/// second, also while the game runs.
pub fn save_bindings(game_dir: &Path, b: &Bindings) -> Result<()> {
    let mut b = b.clone();
    if b.slot > 3 {
        return Err(Error::Other(format!("gamepad slot {} is outside 1 to 4", b.slot + 1)));
    }
    for x in &mut b.bindings {
        if !b.devices.iter().any(|d| d.id == x.device) {
            return Err(Error::Other(format!("binding for {}: device not in the list", x.device)));
        }
        if matches!(x.input, Input::Hat { .. }) != (x.target == hm::Target::Dpad) {
            return Err(Error::Other("a hat switch binds to the whole d-pad, and only a hat switch does".into()));
        }
        x.deadzone = if x.deadzone.is_finite() { x.deadzone.clamp(0.0, 0.5) } else { 0.0 };
        x.curve = if x.curve.is_finite() { x.curve.clamp(0.2, 5.0) } else { 1.0 };
    }
    // Devices no binding uses are dropped.
    let used: Vec<String> = b.bindings.iter().map(|x| x.device.clone()).collect();
    b.devices.retain(|d| used.contains(&d.id));
    let dir = plugin_dir(game_dir);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{}.tmp", hm::BINDINGS_FILE));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&b)?)?;
    std::fs::rename(&tmp, dir.join(hm::BINDINGS_FILE))?;
    Ok(())
}

// ---- plugin state and install ------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Installed {
    pub mod_id: i64,
    pub version: Option<String>,
    pub enabled: bool,
    /// The DLL in the game is the one this CPMX2077 build carries.
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginState {
    /// Version carried by this CPMX2077 build; none when it was built
    /// without the plugin.
    pub bundled: Option<&'static str>,
    pub installed: Option<Installed>,
    /// The DLL is in the game but CPMX2077 has no record of installing it.
    pub unmanaged: bool,
    pub red4ext: bool,
    /// Settings › Mods › HID Input: Mod Settings and redscript present, and
    /// the values saved there.
    pub mod_settings: bool,
    pub redscript: bool,
    pub menu: Option<MenuView>,
    pub menu_error: Option<String>,
    /// XInput, DirectInput and GameInput DLLs the game executable imports,
    /// with the input functions it imports from them.
    pub game_imports: Vec<String>,
    /// From the plugin's last run, with its age in seconds.
    pub status: Option<Status>,
    pub status_age: Option<u64>,
    pub plugin_dir: String,
}

/// The in-game menu's values, for the page.
#[derive(Debug, Clone, Serialize)]
pub struct MenuView {
    pub enabled: bool,
    pub slot: u32,
    pub emulate_pad: bool,
    pub bindings: Vec<MenuBindingView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MenuBindingView {
    pub controller: u32,
    pub target: hm::Target,
    pub input: Input,
    pub invert: bool,
    pub deadzone: f32,
    pub curve: f32,
    pub half: hm::Half,
}

/// Mod Settings' saved HID Input values, if any.
pub fn read_menu(game_dir: &Path) -> std::result::Result<Option<MenuView>, String> {
    let path = resolve_ci(game_dir, menu::USER_INI);
    let text = match std::fs::read(&path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", menu::USER_INI)),
    };
    Ok(menu::parse(&text).map(|m| MenuView {
        enabled: m.enabled,
        slot: m.slot,
        emulate_pad: m.emulate_pad,
        bindings: m
            .bindings
            .into_iter()
            .map(|b| MenuBindingView {
                controller: b.controller,
                target: b.target,
                input: b.input,
                invert: b.invert,
                deadzone: b.deadzone,
                curve: b.curve,
                half: b.half,
            })
            .collect(),
    }))
}

pub fn installed_mod(db: &Db, game_id: i64) -> Result<Option<ModRow>> {
    Ok(db.mods(game_id)?.into_iter().filter(|m| m.source == SOURCE).max_by_key(|m| m.id))
}

fn sha256_bytes(b: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(b))
}

pub fn state(db: &Db, game: &GameRow, bundled: &[u8]) -> Result<PluginState> {
    let game_dir = Path::new(&game.path);
    let dir = plugin_dir(game_dir);
    let dll = dir.join(hm::PLUGIN_DLL);
    let on_disk = dll.is_file().then(|| hash::sha256_file(&dll).ok()).flatten();
    // Current: this build's DLL and script are the ones in the game.
    let script = resolve_ci(game_dir, menu::SCRIPT_PATH);
    let script_current = std::fs::read(&script).is_ok_and(|b| b == menu::SCRIPT.as_bytes());
    let installed = installed_mod(db, game.id)?.map(|m| Installed {
        mod_id: m.id,
        version: m.version.clone(),
        enabled: m.enabled(),
        current: !bundled.is_empty() && script_current && on_disk.as_deref() == Some(sha256_bytes(bundled).as_str()),
    });
    let (menu, menu_error) = match read_menu(game_dir) {
        Ok(m) => (m, None),
        Err(e) => (None, Some(e)),
    };
    let status_path = dir.join(hm::STATUS_FILE);
    let status: Option<Status> = std::fs::read(&status_path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    Ok(PluginState {
        bundled: (!bundled.is_empty()).then_some(hm::PLUGIN_VERSION),
        unmanaged: installed.is_none() && on_disk.is_some(),
        installed,
        red4ext: resolve_ci(game_dir, "red4ext/RED4ext.dll").is_file(),
        mod_settings: resolve_ci(game_dir, menu::MOD_SETTINGS_DLL).is_file(),
        redscript: crate::game::exists_ci(game_dir, "engine/tools/scc.exe") || crate::game::exists_ci(game_dir, "engine/tools/scc_lib.dll"),
        menu,
        menu_error,
        game_imports: game_imports(game_dir),
        status_age: status.as_ref().map(|s| now.saturating_sub(s.updated)),
        status,
        plugin_dir: dir.to_string_lossy().into_owned(),
    })
}

/// Exe path, size and modified time, and the imports found in it.
type ImportCache = Option<(PathBuf, u64, Option<SystemTime>, Vec<String>)>;

/// Input imports of `bin/x64/Cyberpunk2077.exe`, cached by size and time
/// since the file is large.
pub fn game_imports(game_dir: &Path) -> Vec<String> {
    static CACHE: Mutex<ImportCache> = Mutex::new(None);
    let exe = resolve_ci(game_dir, "bin/x64/Cyberpunk2077.exe");
    let Ok(meta) = std::fs::metadata(&exe) else { return Vec::new() };
    let key = (exe.clone(), meta.len(), meta.modified().ok());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, l, t, v)) = cache.as_ref()
        && (p, *l, *t) == (&key.0, key.1, key.2)
    {
        return v.clone();
    }
    let found = read_imports(&exe).unwrap_or_default();
    *cache = Some((key.0, key.1, key.2, found.clone()));
    found
}

fn read_imports(exe: &Path) -> Option<Vec<String>> {
    use pelite::pe64::{Pe, PeFile, imports::Import};
    let map = pelite::FileMap::open(exe).ok()?;
    let file = PeFile::from_bytes(&map).ok()?;
    let mut out = Vec::new();
    for desc in file.imports().ok()? {
        let Ok(dll) = desc.dll_name() else { continue };
        let dll = dll.to_string();
        let lower = dll.to_ascii_lowercase();
        if !(lower.starts_with("xinput") || lower.starts_with("dinput") || lower.starts_with("gameinput") || lower == "hid.dll") {
            continue;
        }
        let names: Vec<String> = desc
            .int()
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|i| match i.ok()? {
                Import::ByName { name, .. } => Some(name.to_string()),
                Import::ByOrdinal { ord } => Some(format!("#{ord}")),
            })
            .collect();
        out.push(if names.is_empty() { dll } else { format!("{dll}: {}", names.join(", ")) });
    }
    Some(out)
}

/// Install (or update) the plugin DLL carried by this CPMX2077 build as a
/// tracked mod: Disable, Enable and Uninstall in the mod list work on it.
/// The bindings and status files are not part of it.
pub fn install(inst: &Installer, game: &GameRow, dll: &[u8]) -> Result<InstallReport> {
    if dll.is_empty() {
        return Err(Error::Other("this CPMX2077 build carries no HID Input plugin".into()));
    }
    if !dll.starts_with(b"MZ") {
        return Err(Error::Other("the carried HID Input plugin is not a Windows DLL".into()));
    }
    let tmp = tempfile::tempdir()?;
    let zip_path = tmp.path().join(format!("cpmx_hid_input-{}.zip", hm::PLUGIN_VERSION));
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path)?);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file(format!("{}/{}", hm::PLUGIN_DIR, hm::PLUGIN_DLL), opts).map_err(|e| Error::Other(e.to_string()))?;
        z.write_all(dll)?;
        z.start_file(menu::SCRIPT_PATH, opts).map_err(|e| Error::Other(e.to_string()))?;
        z.write_all(menu::SCRIPT.as_bytes())?;
        z.finish().map_err(|e| Error::Other(e.to_string()))?;
    }
    let p = inst.prepare(game, &zip_path)?;
    let meta = NewMod {
        name: MOD_NAME.into(),
        version: Some(hm::PLUGIN_VERSION.into()),
        source: SOURCE.into(),
        archive_name: p.archive_name.clone(),
        category: Some("Input".into()),
        ..Default::default()
    };
    let opts = InstallOptions { meta, overwrite: false, fomod_choices: None };
    let result = match installed_mod(inst.db, game.id)? {
        Some(old) => inst.finish_replacing(game, &p, opts, old.id),
        None => inst.finish(game, &p, opts),
    };
    if result.is_err() {
        inst.discard(&p);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use cp2077mm_hidmap::{Binding, Device, Half, PAGE_GENERIC, Target};

    #[test]
    fn kinds_from_names_and_inputs() {
        assert_eq!(guess_kind("Logitech G29 Driving Force Racing Wheel", &[], 20, 1), "wheel");
        assert_eq!(guess_kind("Thrustmaster T-Rudder", &[], 0, 0), "pedals");
        assert_eq!(guess_kind("VKB-Sim Gladiator NXT", &[], 30, 1), "stick");
        assert_eq!(guess_kind("Generic USB Device", &[usage(PAGE_SIMULATION, 0xC4)], 0, 0), "pedals");
        assert_eq!(guess_kind("Generic USB Device", &[usage(PAGE_GENERIC, 0x30)], 32, 0), "generic");
        assert_eq!(guess_kind("Arduino Leonardo", &[], 24, 0), "buttonbox");
        assert_eq!(guess_kind("Xbox Wireless Controller", &[], 11, 1), "gamepad");
    }

    fn sample() -> Bindings {
        Bindings {
            devices: vec![Device {
                id: "046d:c24f".into(),
                vid: 0x46d,
                pid: 0xc24f,
                instance: 0,
                name: "Pedals".into(),
                hid_axes: vec![],
                kind: Some("pedals".into()),
            }],
            bindings: vec![Binding {
                device: "046d:c24f".into(),
                input: Input::Axis { index: 2, usage: None, occurrence: 0 },
                target: Target::RightTrigger,
                invert: false,
                deadzone: 0.9,
                curve: 1.0,
                half: Half::Full,
            }],
            ..Bindings::default()
        }
    }

    #[test]
    fn bindings_save_and_load() {
        let game = tempfile::tempdir().unwrap();
        assert_eq!(load_bindings(game.path()).unwrap(), Bindings::default());
        let mut b = sample();
        b.devices.push(Device { id: "1234:5678".into(), ..b.devices[0].clone() });
        save_bindings(game.path(), &b).unwrap();
        let back = load_bindings(game.path()).unwrap();
        assert_eq!(back.bindings[0].deadzone, 0.5, "clamped");
        assert_eq!(back.devices.len(), 1, "unused device dropped");
        assert!(game.path().join(hm::PLUGIN_DIR).join(hm::BINDINGS_FILE).is_file());
        // A binding to an unknown device, or a hat on a button, is refused.
        let mut bad = sample();
        bad.devices.clear();
        assert!(save_bindings(game.path(), &bad).is_err());
        let mut bad = sample();
        bad.bindings[0].target = Target::Dpad;
        assert!(save_bindings(game.path(), &bad).is_err());
        let mut bad = sample();
        bad.slot = 4;
        assert!(save_bindings(game.path(), &bad).is_err());
    }

    #[test]
    fn install_records_the_plugin_as_a_mod() {
        let (root, db, game) = crate::reshade::tests::setup_game();
        let inst = crate::reshade::tests::installer(&db, root.path());
        let game_dir = PathBuf::from(&game.path);
        std::fs::write(game_dir.join("bin/x64/Cyberpunk2077.exe"), b"not a PE").unwrap();
        let dll = b"MZ fake plugin".to_vec();
        assert!(install(&inst, &game, b"").is_err());
        assert!(install(&inst, &game, b"ELF").is_err());
        let r = install(&inst, &game, &dll).unwrap();
        assert_eq!(r.files_installed, 2, "the DLL and the in-game menu script");
        assert_eq!(std::fs::read_to_string(game_dir.join(menu::SCRIPT_PATH)).unwrap(), menu::SCRIPT);
        let s = state(&db, &game, &dll).unwrap();
        let i = s.installed.unwrap();
        assert!(i.current && i.enabled);
        assert_eq!(i.version.as_deref(), Some(hm::PLUGIN_VERSION));
        assert!(!s.unmanaged && !s.red4ext && s.game_imports.is_empty() && s.status.is_none());
        assert!(!s.mod_settings && s.menu.is_none());
        // An edited or missing script means the install is not current.
        std::fs::write(game_dir.join(menu::SCRIPT_PATH), "// edited").unwrap();
        assert!(!state(&db, &game, &dll).unwrap().installed.unwrap().current);
        // Values saved in Settings > Mods are read from Mod Settings' user.ini.
        let ini = game_dir.join(menu::USER_INI);
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(game_dir.join(menu::MOD_SETTINGS_DLL), b"MZ").unwrap();
        std::fs::write(&ini, "[CPMXHIDInputSettings]\ngamepadSlot = 2\nrightTriggerController = 1\nrightTriggerInput = 2\n").unwrap();
        let s = state(&db, &game, &dll).unwrap();
        let m = s.menu.unwrap();
        assert!(s.mod_settings && m.slot == 1 && m.bindings.len() == 1);
        assert_eq!(m.bindings[0].input, Input::Axis { index: 2, usage: None, occurrence: 0 });
        // A newer build's DLL replaces the old record.
        let newer = b"MZ newer plugin".to_vec();
        assert!(!state(&db, &game, &newer).unwrap().installed.unwrap().current);
        install(&inst, &game, &newer).unwrap();
        assert_eq!(db.mods(game.id).unwrap().iter().filter(|m| m.source == SOURCE).count(), 1);
        assert_eq!(std::fs::read(game_dir.join(hm::PLUGIN_DIR).join(hm::PLUGIN_DLL)).unwrap(), newer);
        // The plugin's status file is read back.
        let status = Status { plugin_version: "0.1.0".into(), get_state_calls: 7, ..Default::default() };
        std::fs::write(game_dir.join(hm::PLUGIN_DIR).join(hm::STATUS_FILE), serde_json::to_vec(&status).unwrap()).unwrap();
        assert_eq!(state(&db, &game, &newer).unwrap().status.unwrap().get_state_calls, 7);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn lists_joysticks_from_sysfs() {
        let root = tempfile::tempdir().unwrap();
        let sys = root.path().join("sys");
        let udev = root.path().join("udev");
        std::fs::create_dir_all(&udev).unwrap();
        let node = |n: u32, name: &str, vendor: &str, abs: &str, key: &str, joystick: bool, desc: Option<&[u8]>| {
            let ev = sys.join(format!("event{n}"));
            let dev = ev.join("device");
            std::fs::create_dir_all(dev.join("id")).unwrap();
            std::fs::create_dir_all(dev.join("capabilities")).unwrap();
            std::fs::write(ev.join("dev"), format!("13:{}\n", 64 + n)).unwrap();
            std::fs::write(dev.join("name"), format!("{name}\n")).unwrap();
            std::fs::write(dev.join("id/vendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(dev.join("id/product"), "c24f\n").unwrap();
            std::fs::write(dev.join("capabilities/abs"), format!("{abs}\n")).unwrap();
            std::fs::write(dev.join("capabilities/key"), format!("{key}\n")).unwrap();
            if let Some(d) = desc {
                std::fs::create_dir_all(dev.join("device")).unwrap();
                std::fs::write(dev.join("device/report_descriptor"), d).unwrap();
            }
            std::fs::write(udev.join(format!("c13:{}", 64 + n)), if joystick { "E:ID_INPUT=1\nE:ID_INPUT_JOYSTICK=1\n" } else { "E:ID_INPUT_KEYBOARD=1\n" }).unwrap();
        };
        // Pedals: Accelerator and Brake (ABS_GAS 9, ABS_BRAKE 10) and X, from a descriptor.
        let desc: &[u8] = &[
            0x05, 0x01, 0x09, 0x04, 0xA1, 0x01, 0x05, 0x02, 0x09, 0xC4, 0x09, 0xC5, 0x15, 0x00, 0x26, 0xFF, 0x03, 0x75,
            0x10, 0x95, 0x02, 0x81, 0x02, 0x05, 0x01, 0x09, 0x30, 0x75, 0x10, 0x95, 0x01, 0x81, 0x02, 0xC0,
        ];
        node(3, "USB Pedals", "046d", &format!("{:x}", 1u64 | 1 << 9 | 1 << 10), "0", true, Some(desc));
        node(1, "AT Keyboard", "0001", "0", "fffffffffffffffe", false, None);
        // A second, identical set and a hat on the first stick.
        node(12, "USB Pedals", "046d", &format!("{:x}", 1u64 | 1 << 9 | 1 << 10), "0", true, Some(desc));
        node(7, "Flight Stick", "044f", &format!("{:x}", 3u64 | 1 << 16 | 1 << 17), "ffff 0 0 0 0", true, None);
        let devs = evdev::scan(&sys, &udev, Path::new("/dev/input"));
        let views: Vec<&DeviceView> = devs.iter().map(|d| &d.view).collect();
        assert_eq!(views.len(), 3, "keyboard left out");
        let pedals = views[0];
        assert_eq!((pedals.id.as_str(), pedals.kind), ("046d:c24f", "pedals"));
        assert_eq!(pedals.hid_axes, vec![usage(2, 0xC4), usage(2, 0xC5), usage(1, 0x30)]);
        let labels: Vec<&str> = pedals.axes.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["X axis", "Accelerator", "Brake"], "evdev code order");
        assert_eq!(pedals.axes[2].usage, Some(usage(2, 0xC5)));
        let stick = views[1];
        assert_eq!((stick.id.as_str(), stick.hats, stick.buttons, stick.axes.len()), ("044f:c24f", 1, 16, 2));
        assert_eq!(stick.kind, "stick");
        assert!(stick.hid_axes.is_empty() && stick.axes[0].usage.is_none());
        assert_eq!(views[2].id, "046d:c24f#1");
        assert_eq!(evdev::bits("3 1"), vec![0, 64, 65]);
    }
}
