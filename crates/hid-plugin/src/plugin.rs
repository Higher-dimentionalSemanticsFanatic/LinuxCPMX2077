//! The RED4ext entry points, the XInput hooks and the device threads.
//!
//! RED4ext loads the DLL from `red4ext/plugins/cpmx_hid_input/` and calls
//! `Supports`, `Query` and `Main`. `Main` hooks XInputGetState (and
//! XInputGetStateEx, XInputGetCapabilities) in every XInput DLL through
//! RED4ext's hooking API, then a background thread watches the bindings
//! file, opens the bound devices and writes the status file. One thread per
//! open device waits for its input reports. The hooks add the bound inputs
//! to the game's reads of the bound gamepad slot.

use std::collections::HashSet;
use std::ffi::{CString, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cp2077mm_hidmap::{self as hm, Active, Bindings, Live, Pad, Status, StatusDevice, win};
use windows_sys::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_SUCCESS, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleFileNameW, GetModuleHandleExW,
    GetModuleHandleW, GetProcAddress, LoadLibraryW,
};
use windows_sys::Win32::UI::Input::XboxController::{XINPUT_CAPABILITIES, XINPUT_GAMEPAD, XINPUT_STATE};

// ---- RED4ext ABI (RED4ext.SDK, include/RED4ext/Api/v1) -----------------------

#[repr(C)]
pub struct SemVer {
    major: u8,
    minor: u16,
    patch: u32,
    prerelease_type: u32,
    prerelease_number: u32,
}

#[repr(C)]
pub struct FileVer {
    major: u16,
    minor: u16,
    build: u16,
    revision: u16,
}

#[repr(C)]
pub struct PluginInfo {
    sdk: SemVer,
    name: *const u16,
    author: *const u16,
    version: SemVer,
    runtime: FileVer,
}

/// Trace, Debug, Info, Warn, Error and Critical, each as plain, printf,
/// wide and wide printf.
#[repr(C)]
pub struct Logger {
    fns: [*const c_void; 24],
}

#[repr(C)]
pub struct Hooking {
    attach: unsafe extern "C" fn(HMODULE, *mut c_void, *mut c_void, *mut *mut c_void) -> bool,
    detach: unsafe extern "C" fn(HMODULE, *mut c_void) -> bool,
}

#[repr(C)]
pub struct Sdk {
    runtime: *const SemVer,
    logger: *const Logger,
    hooking: *const Hooking,
    game_states: *const c_void,
    scripts: *const c_void,
}

/// `RED4EXT_API_VERSION_1_COMPAT_0`: API v1, accepted by older loaders too.
const API_VERSION: u32 = 0;

const fn wide<const N: usize>(s: &str) -> [u16; N] {
    let b = s.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    while i < b.len() {
        out[i] = b[i] as u16;
        i += 1;
    }
    out
}

static NAME: [u16; hm::PLUGIN_NAME.len() + 1] = wide(hm::PLUGIN_NAME);
const AUTHOR_STR: &str = "CPMX2077";
static AUTHOR: [u16; AUTHOR_STR.len() + 1] = wide(AUTHOR_STR);
const VERSION: &str = hm::PLUGIN_VERSION;

const fn version_part(n: usize) -> u32 {
    let s = VERSION.as_bytes();
    let (mut i, mut part, mut v) = (0, 0, 0u32);
    while i < s.len() {
        let c = s[i];
        if c == b'.' {
            if part == n {
                return v;
            }
            part += 1;
            v = 0;
        } else if c.is_ascii_digit() {
            v = v * 10 + (c - b'0') as u32;
        } else {
            break;
        }
        i += 1;
    }
    if part == n { v } else { 0 }
}

/// RED4ext asks which API the plugin speaks.
#[unsafe(no_mangle)]
pub extern "C" fn Supports() -> u32 {
    API_VERSION
}

/// RED4ext asks for the plugin's name, author and versions.
///
/// # Safety
/// `info` points to RED4ext's `PluginInfo`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Query(info: *mut PluginInfo) {
    if info.is_null() {
        return;
    }
    let semver = |major: u32, minor: u32, patch: u32| SemVer {
        major: major as u8,
        minor: minor as u16,
        patch,
        prerelease_type: 0,
        prerelease_number: 0,
    };
    unsafe {
        // RED4EXT_V1_SDK_VERSION_1_0_0_COMPAT_0_5_0 and RED4EXT_V1_RUNTIME_VERSION_INDEPENDENT:
        // no game offsets are used, so any game version.
        (*info).sdk = semver(0, 5, 0);
        (*info).name = NAME.as_ptr();
        (*info).author = AUTHOR.as_ptr();
        (*info).version = semver(version_part(0), version_part(1), version_part(2));
        (*info).runtime = FileVer { major: u16::MAX, minor: u16::MAX, build: u16::MAX, revision: u16::MAX };
    }
}

/// RED4ext loads (reason 0) or unloads (1) the plugin.
///
/// # Safety
/// `sdk` points to RED4ext's `Sdk`, valid while the plugin is loaded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Main(handle: HMODULE, reason: u8, sdk: *const Sdk) -> bool {
    if sdk.is_null() {
        return false;
    }
    let sdk = unsafe { &*sdk };
    match reason {
        0 => unsafe { load(handle, sdk) },
        _ => unsafe { unload(handle, sdk) },
    }
    true
}

// ---- state shared by the hooks and the threads -------------------------------

struct Connected {
    info: win::Info,
    connected: bool,
}

#[derive(Default)]
struct Engine {
    cfg: Bindings,
    cfg_error: Option<String>,
    /// Devices opened since the game started; a slot is reused when the
    /// same device comes back.
    devices: Vec<Connected>,
    live: Vec<Live>,
    active: Vec<Active>,
    /// Every game controller seen by the last scan.
    seen: Vec<win::Info>,
    /// Per XInput slot: the last pad returned and how often it changed.
    last: [Pad; 4],
    changes: [u32; 4],
}

impl Engine {
    /// Match every binding to a connected device and input.
    fn rebuild(&mut self) {
        let mut active = Vec::new();
        for b in &self.cfg.bindings {
            let Some(dev) = self.cfg.devices.iter().find(|d| d.id == b.device) else { continue };
            let Some((i, c)) = self.devices.iter().enumerate().find(|(_, c)| {
                c.connected && c.info.vid == dev.vid && c.info.pid == dev.pid && c.info.instance == dev.instance
            }) else {
                continue;
            };
            if let Some(slot) = hm::resolve(&b.input, dev, &c.info.layout) {
                active.push(Active { binding: b.clone(), device: i, slot });
            }
        }
        self.active = active;
    }

    fn wanted(&self, info: &win::Info) -> bool {
        self.cfg.devices.iter().any(|d| {
            d.vid == info.vid && d.pid == info.pid && d.instance == info.instance && self.cfg.bindings.iter().any(|b| b.device == d.id)
        })
    }

    fn missing(&self) -> Vec<String> {
        self.cfg
            .devices
            .iter()
            .filter(|d| self.cfg.bindings.iter().any(|b| b.device == d.id))
            .filter(|d| {
                !self.devices.iter().any(|c| c.connected && c.info.vid == d.vid && c.info.pid == d.pid && c.info.instance == d.instance)
            })
            .map(|d| if d.name.is_empty() { d.id.clone() } else { format!("{} ({})", d.name, d.id) })
            .collect()
    }
}

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();

fn engine() -> MutexGuard<'static, Engine> {
    ENGINE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

static GET_STATE_CALLS: AtomicU64 = AtomicU64::new(0);
static MERGED_READS: AtomicU64 = AtomicU64::new(0);
static EMULATED_READS: AtomicU64 = AtomicU64::new(0);
static STOPPING: AtomicBool = AtomicBool::new(false);
static HOOKS: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());
static HOOK_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

// ---- XInput hooks --------------------------------------------------------------

type GetState = unsafe extern "system" fn(u32, *mut XINPUT_STATE) -> u32;
type GetCaps = unsafe extern "system" fn(u32, u32, *mut XINPUT_CAPABILITIES) -> u32;

/// Every XInput DLL a game may use; each one the game loads gets hooked.
const MODULES: [&str; 5] = ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll", "xinput1_2.dll", "xinput1_1.dll"];

#[allow(clippy::declare_interior_mutable_const)]
const NULL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIG_STATE: [AtomicPtr<c_void>; 5] = [NULL; 5];
static ORIG_STATE_EX: [AtomicPtr<c_void>; 5] = [NULL; 5];
static ORIG_CAPS: [AtomicPtr<c_void>; 5] = [NULL; 5];

unsafe extern "system" fn get_state<const I: usize>(user: u32, state: *mut XINPUT_STATE) -> u32 {
    unsafe { on_state(&ORIG_STATE[I], user, state) }
}

unsafe extern "system" fn get_state_ex<const I: usize>(user: u32, state: *mut XINPUT_STATE) -> u32 {
    unsafe { on_state(&ORIG_STATE_EX[I], user, state) }
}

unsafe extern "system" fn get_caps<const I: usize>(user: u32, flags: u32, caps: *mut XINPUT_CAPABILITIES) -> u32 {
    unsafe { on_caps(&ORIG_CAPS[I], user, flags, caps) }
}

const STATE_DETOURS: [GetState; 5] = [get_state::<0>, get_state::<1>, get_state::<2>, get_state::<3>, get_state::<4>];
const STATE_EX_DETOURS: [GetState; 5] =
    [get_state_ex::<0>, get_state_ex::<1>, get_state_ex::<2>, get_state_ex::<3>, get_state_ex::<4>];
const CAPS_DETOURS: [GetCaps; 5] = [get_caps::<0>, get_caps::<1>, get_caps::<2>, get_caps::<3>, get_caps::<4>];

fn to_pad(g: &XINPUT_GAMEPAD) -> Pad {
    Pad {
        buttons: g.wButtons,
        left_trigger: g.bLeftTrigger,
        right_trigger: g.bRightTrigger,
        lx: g.sThumbLX,
        ly: g.sThumbLY,
        rx: g.sThumbRX,
        ry: g.sThumbRY,
    }
}

/// Whether the plugin answers for `user` when no real gamepad is there.
fn emulating(e: &Engine, user: u32) -> bool {
    e.cfg.enabled && e.cfg.emulate_pad && user == e.cfg.slot && !e.active.is_empty()
}

unsafe fn on_state(orig: &AtomicPtr<c_void>, user: u32, state: *mut XINPUT_STATE) -> u32 {
    GET_STATE_CALLS.fetch_add(1, Ordering::Relaxed);
    let f = orig.load(Ordering::Acquire);
    let r = if f.is_null() {
        ERROR_DEVICE_NOT_CONNECTED
    } else {
        unsafe { std::mem::transmute::<*mut c_void, GetState>(f)(user, state) }
    };
    if state.is_null() || STOPPING.load(Ordering::Relaxed) {
        return r;
    }
    let mut e = engine();
    if !e.cfg.enabled || user != e.cfg.slot || user > 3 {
        return r;
    }
    let (real, packet) = if r == ERROR_SUCCESS {
        let s = unsafe { &*state };
        (to_pad(&s.Gamepad), s.dwPacketNumber)
    } else if r == ERROR_DEVICE_NOT_CONNECTED && emulating(&e, user) {
        EMULATED_READS.fetch_add(1, Ordering::Relaxed);
        (Pad::default(), 0)
    } else {
        return r;
    };
    let c = hm::contribution(&e.active, &e.live);
    if !c.is_neutral() {
        MERGED_READS.fetch_add(1, Ordering::Relaxed);
    }
    let pad = c.apply(real);
    // Games skip a read whose packet number did not change, so count our
    // own changes into it.
    let slot = user as usize;
    if pad != e.last[slot] {
        e.last[slot] = pad;
        e.changes[slot] = e.changes[slot].wrapping_add(1);
    }
    let s = unsafe { &mut *state };
    s.dwPacketNumber = packet.wrapping_add(e.changes[slot]);
    s.Gamepad = XINPUT_GAMEPAD {
        wButtons: pad.buttons,
        bLeftTrigger: pad.left_trigger,
        bRightTrigger: pad.right_trigger,
        sThumbLX: pad.lx,
        sThumbLY: pad.ly,
        sThumbRX: pad.rx,
        sThumbRY: pad.ry,
    };
    ERROR_SUCCESS
}

unsafe fn on_caps(orig: &AtomicPtr<c_void>, user: u32, flags: u32, caps: *mut XINPUT_CAPABILITIES) -> u32 {
    let f = orig.load(Ordering::Acquire);
    let r = if f.is_null() {
        ERROR_DEVICE_NOT_CONNECTED
    } else {
        unsafe { std::mem::transmute::<*mut c_void, GetCaps>(f)(user, flags, caps) }
    };
    if r != ERROR_DEVICE_NOT_CONNECTED || caps.is_null() || STOPPING.load(Ordering::Relaxed) || !emulating(&engine(), user) {
        return r;
    }
    let c = unsafe { &mut *caps };
    c.Type = 1; // XINPUT_DEVTYPE_GAMEPAD
    c.SubType = 1; // XINPUT_DEVSUBTYPE_GAMEPAD
    c.Flags = 0;
    c.Gamepad = XINPUT_GAMEPAD {
        wButtons: 0xF3FF,
        bLeftTrigger: 255,
        bRightTrigger: 255,
        sThumbLX: -64,
        sThumbLY: -64,
        sThumbRX: -64,
        sThumbRY: -64,
    };
    c.Vibration.wLeftMotorSpeed = 0;
    c.Vibration.wRightMotorSpeed = 0;
    ERROR_SUCCESS
}

fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn log(sdk: &Sdk, handle: HMODULE, level: usize, msg: &str) {
    // Info 8, Warn 12, Error 16: the plain (char*) variants.
    let Ok(text) = CString::new(msg.replace('\0', " ")) else { return };
    if sdk.logger.is_null() {
        return;
    }
    let f = unsafe { (*sdk.logger).fns[level] };
    if f.is_null() {
        return;
    }
    let f: unsafe extern "C" fn(HMODULE, *const u8) = unsafe { std::mem::transmute(f) };
    unsafe { f(handle, text.as_ptr().cast()) };
}

const INFO: usize = 8;
const WARN: usize = 12;

/// Hook the XInput functions of every XInput DLL, loading the ones the game
/// hasn't loaded yet so a later load by the game gets the hooked copy.
unsafe fn hook_xinput(handle: HMODULE, sdk: &Sdk) {
    if sdk.hooking.is_null() {
        HOOK_ERRORS.lock().unwrap_or_else(|e| e.into_inner()).push("RED4ext gave no hooking API".into());
        return;
    }
    let hooking = unsafe { &*sdk.hooking };
    let mut done: HashSet<usize> = HashSet::new();
    for (i, name) in MODULES.iter().enumerate() {
        let w = wide_z(name);
        let mut m = unsafe { GetModuleHandleW(w.as_ptr()) };
        if m.is_null() {
            m = unsafe { LoadLibraryW(w.as_ptr()) };
        }
        if m.is_null() {
            continue;
        }
        let targets: [(&str, *const u8, *mut c_void, &AtomicPtr<c_void>); 3] = [
            ("XInputGetState", c"XInputGetState".as_ptr().cast(), STATE_DETOURS[i] as *mut c_void, &ORIG_STATE[i]),
            // Ordinal 100: XInputGetStateEx, which adds the guide button.
            ("XInputGetStateEx", 100 as *const u8, STATE_EX_DETOURS[i] as *mut c_void, &ORIG_STATE_EX[i]),
            ("XInputGetCapabilities", c"XInputGetCapabilities".as_ptr().cast(), CAPS_DETOURS[i] as *mut c_void, &ORIG_CAPS[i]),
        ];
        for (fname, proc, detour, orig) in targets {
            let Some(target) = (unsafe { GetProcAddress(m, proc) }) else { continue };
            let target = target as *mut c_void;
            // A forwarded export is the same function: hook it once.
            if !done.insert(target as usize) {
                continue;
            }
            let label = format!("{name}!{fname}");
            if unsafe { (hooking.attach)(handle, target, detour, orig.as_ptr()) } {
                HOOKS.lock().unwrap_or_else(|e| e.into_inner()).push((label.clone(), target as usize));
                log(sdk, handle, INFO, &format!("hooked {label}"));
            } else {
                HOOK_ERRORS.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{label}: RED4ext could not attach the hook"));
                log(sdk, handle, WARN, &format!("could not hook {label}"));
            }
        }
    }
}

fn module_dir(handle: HMODULE) -> Option<PathBuf> {
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetModuleFileNameW(handle, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let path = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    path.parent().map(Path::to_path_buf)
}

unsafe fn load(handle: HMODULE, sdk: &Sdk) {
    // Device threads outlive RED4ext's unload at game exit: keep the DLL
    // mapped until the process ends.
    let mut pinned: HMODULE = std::ptr::null_mut();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            Main as *const u16,
            &mut pinned,
        )
    };
    let Some(dir) = module_dir(handle) else {
        log(sdk, handle, WARN, "plugin folder not found; nothing bound");
        return;
    };
    {
        let mut e = engine();
        load_config(&mut e, &dir.join(hm::BINDINGS_FILE));
        let n = e.cfg.bindings.len();
        drop(e);
        log(sdk, handle, INFO, &format!("{} {VERSION}: {n} binding(s) in {}", hm::PLUGIN_NAME, hm::BINDINGS_FILE));
    }
    unsafe { hook_xinput(handle, sdk) };
    let started = unix_now();
    let _ = std::thread::Builder::new().name("cpmx-hid-monitor".into()).spawn(move || monitor(dir, started));
}

unsafe fn unload(handle: HMODULE, sdk: &Sdk) {
    STOPPING.store(true, Ordering::Relaxed);
    if sdk.hooking.is_null() {
        return;
    }
    let hooks = std::mem::take(&mut *HOOKS.lock().unwrap_or_else(|e| e.into_inner()));
    for (_, target) in hooks {
        unsafe { ((*sdk.hooking).detach)(handle, target as *mut c_void) };
    }
}

// ---- bindings, devices and status ----------------------------------------------

fn load_config(e: &mut Engine, path: &Path) {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Bindings>(&bytes) {
            Ok(cfg) => {
                e.cfg = cfg;
                e.cfg_error = None;
            }
            // Keep the previous bindings until the file parses again.
            Err(err) => e.cfg_error = Some(format!("{}: {err}", hm::BINDINGS_FILE)),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            e.cfg = Bindings::default();
            e.cfg_error = None;
        }
        Err(err) => e.cfg_error = Some(format!("{}: {err}", hm::BINDINGS_FILE)),
    }
    e.cfg.slot = e.cfg.slot.min(3);
    e.rebuild();
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Open the bound devices that are connected and not yet open.
fn scan() {
    let found = win::list();
    let mut e = engine();
    let mut opened = Vec::new();
    for info in &found {
        if !e.wanted(info) || e.devices.iter().any(|c| c.connected && c.info.path == info.path) {
            continue;
        }
        let Some(reader) = win::Reader::open(&info.path) else { continue };
        let mut info = info.clone();
        info.layout = reader.layout();
        let idle = reader.idle();
        let slot = match e.devices.iter().position(|c| c.info.path == info.path) {
            Some(i) => {
                e.devices[i] = Connected { info, connected: true };
                e.live[i] = idle;
                i
            }
            None => {
                e.devices.push(Connected { info, connected: true });
                e.live.push(idle);
                e.devices.len() - 1
            }
        };
        opened.push((slot, reader));
    }
    e.seen = found;
    if !opened.is_empty() {
        e.rebuild();
    }
    drop(e);
    for (slot, reader) in opened {
        let _ = std::thread::Builder::new().name(format!("cpmx-hid-{slot}")).spawn(move || read_device(slot, reader));
    }
}

fn read_device(slot: usize, mut reader: win::Reader) {
    let mut live = reader.idle();
    while !STOPPING.load(Ordering::Relaxed) && reader.read(&mut live) {
        let mut e = engine();
        if let Some(l) = e.live.get_mut(slot) {
            l.clone_from(&live);
        }
    }
    let mut e = engine();
    if let Some(c) = e.devices.get_mut(slot) {
        c.connected = false;
    }
    if let Some(l) = e.live.get_mut(slot) {
        *l = reader.idle();
    }
    e.rebuild();
}

fn status(started: u64) -> Status {
    let e = engine();
    let devices = e
        .seen
        .iter()
        .map(|i| StatusDevice {
            id: hm::device_id(i.vid, i.pid, i.instance),
            name: i.name.clone(),
            vid: i.vid,
            pid: i.pid,
            axes: i.layout.axes.clone(),
            buttons: i.layout.buttons,
            hats: i.layout.hats,
            bound: e.devices.iter().enumerate().any(|(n, c)| c.connected && c.info.path == i.path && e.active.iter().any(|a| a.device == n)),
        })
        .collect();
    Status {
        plugin_version: VERSION.into(),
        started,
        updated: unix_now(),
        hooks: HOOKS.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(l, _)| l.clone()).collect(),
        hook_errors: HOOK_ERRORS.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        get_state_calls: GET_STATE_CALLS.load(Ordering::Relaxed),
        merged_reads: MERGED_READS.load(Ordering::Relaxed),
        emulated_reads: EMULATED_READS.load(Ordering::Relaxed),
        config_error: e.cfg_error.clone(),
        enabled: e.cfg.enabled,
        bindings: e.cfg.bindings.len(),
        active_bindings: e.active.len(),
        devices,
        missing: e.missing(),
    }
}

fn write_status(dir: &Path, started: u64) {
    let Ok(text) = serde_json::to_vec_pretty(&status(started)) else { return };
    let tmp = dir.join(format!("{}.tmp", hm::STATUS_FILE));
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(hm::STATUS_FILE));
    }
}

/// Reload the bindings when the file changes, open bound devices as they
/// connect, and write the status file.
fn monitor(dir: PathBuf, started: u64) {
    let cfg_path = dir.join(hm::BINDINGS_FILE);
    let mut cfg_time = modified(&cfg_path);
    let mut last_scan: Option<Instant> = None;
    let mut last_status: Option<Instant> = None;
    while !STOPPING.load(Ordering::Relaxed) {
        let t = modified(&cfg_path);
        let mut rescan = false;
        if t != cfg_time {
            cfg_time = t;
            load_config(&mut engine(), &cfg_path);
            rescan = true;
        }
        let missing = !engine().missing().is_empty();
        let due = last_scan.is_none_or(|s| s.elapsed() >= Duration::from_secs(if missing { 3 } else { 30 }));
        if rescan || due {
            scan();
            last_scan = Some(Instant::now());
        }
        if last_status.is_none_or(|s| s.elapsed() >= Duration::from_secs(5)) || rescan {
            write_status(&dir, started);
            last_status = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
