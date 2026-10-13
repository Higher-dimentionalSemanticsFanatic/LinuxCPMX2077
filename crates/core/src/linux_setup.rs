//! What the game needs on Linux before mods load, checked and fixed in
//! place: the Visual C++ runtime in the Wine/Proton prefix, the `winmm` and
//! `version` DLL overrides that let CET and RED4ext load, ReShade's DLL
//! override and Microsoft's shader compiler, `-modded` for REDmod and
//! `--launcher-skip` (with the REDmod switch that replaces the REDprelauncher), mod
//! folders that exist twice with different capitalisation, what HDR
//! needs (see [`crate::hdr`]), OptiScaler's DLL override and the
//! `VKD3D_SHADER_OVERRIDE` launch option for FSR 4.1.1 shader overrides (see
//! [`crate::optiscaler`]).
//!
//! Every fix is offered, never applied on its own: the user confirms it, a
//! record of what changed is kept under the app's data dir, and Undo puts
//! things back. Steam's settings are only written while Steam is closed,
//! because Steam rewrites `localconfig.vdf` when it quits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::activity::{self, Kind};
use crate::desktop::user_command;
use crate::game::{self, GameInstall, Store};
use crate::hdr;
use crate::install::{KNOWN_ROOTS, resolve_ci};
use crate::{Error, Result, STEAM_APP_ID, vdf};

/// The DLL overrides CET (`version`) and RED4ext (`winmm`) need under Wine.
pub const OVERRIDE_VALUE: &str = "winmm,version=n,b";
/// Oldest `msvcp140.dll` (major, minor) that current CET and RED4ext builds
/// load with: MSVC 17.10 changed std::mutex in a way older runtimes crash on
/// (error 998 at startup).
pub const MIN_VC_RUNTIME: (u16, u16) = (14, 40);
/// File name starts of everything the redistributable may write, for the backup.
const VC_FILE_STEMS: &[&str] = &["concrt140", "msvcp140", "vcruntime140", "vcomp140", "vcamp140", "vccorlib140", "mfc140", "mfcm140"];
const SYSTEM_DIRS: &[&str] = &["drive_c/windows/system32", "drive_c/windows/syswow64"];
const PROTONTRICKS_FLATPAK: &str = "com.github.Matoking.protontricks";

pub const FIX_VC: &str = "vc-runtime";
pub const FIX_D3DCOMPILER: &str = "d3dcompiler";
pub const FIX_LAUNCH: &str = "launch-options";
pub const FIX_OVERRIDES: &str = "dll-overrides";
pub const FIX_CASE: &str = "case-folders";
pub const FIX_HDR: &str = "hdr";
/// HDR through gamescope, or back to Wine's Wayland driver, when both work.
pub const FIX_HDR_GAMESCOPE: &str = "hdr-gamescope";
pub const FIX_HDR_WAYLAND: &str = "hdr-wayland";
pub const FIX_FSR4: &str = "fsr4-overrides";

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ok,
    Problem,
}

/// A button the UI shows, with the question it asks first. `blocked` says
/// why it can't run right now (Steam open, a tool missing).
#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub label: String,
    pub confirm: String,
    pub blocked: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: String,
    pub title: String,
    pub state: State,
    pub detail: String,
    pub fix: Option<Action>,
    /// Present once a fix has been applied and can be taken back.
    pub undo: Option<Action>,
    /// Text the user can copy and set by hand instead (launch options).
    pub copy: Option<String>,
    /// A second way to fix it (HDR through gamescope instead of Wayland),
    /// applied with `alt_id` in place of `id`.
    pub alt: Option<Action>,
    pub alt_id: Option<String>,
}

/// A way to install Windows components into a prefix.
#[derive(Debug, Clone, PartialEq)]
pub enum Tricks {
    Protontricks(PathBuf),
    ProtontricksFlatpak,
    Winetricks(PathBuf),
}

/// What's running and installed on this machine. Tests build one by hand.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub steam_running: bool,
    pub game_running: bool,
    pub protontricks: Option<Tricks>,
    pub winetricks: Option<PathBuf>,
    pub cabextract: bool,
    pub hdr: hdr::Probe,
}

impl Probe {
    pub fn system() -> Self {
        let procs = processes();
        let protontricks = which("protontricks").map(Tricks::Protontricks).or_else(|| {
            let ok = user_command("flatpak")
                .args(["info", PROTONTRICKS_FLATPAK])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            ok.then_some(Tricks::ProtontricksFlatpak)
        });
        Probe {
            steam_running: procs.iter().any(|(comm, _)| comm == "steam") || crate::winsys::process_running("steam.exe"),
            game_running: procs.iter().any(|(_, cmd)| cmd.to_ascii_lowercase().contains("cyberpunk2077.exe")),
            protontricks,
            winetricks: which("winetricks"),
            cabextract: which("cabextract").is_some(),
            hdr: hdr::Probe::system(),
        }
    }
}

/// `(comm, cmdline)` of every process we can read.
fn processes() -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir("/proc") else { return vec![] };
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|e| {
            let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
            let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
            Some((comm.trim().to_string(), String::from_utf8_lossy(&cmd).replace('\0', " ")))
        })
        .collect()
}

pub(crate) fn on_path(name: &str) -> bool {
    which(name).is_some()
}

fn which(name: &str) -> Option<PathBuf> {
    let appdir = std::env::var("APPDIR").ok().filter(|a| !a.is_empty());
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .filter(|d| !d.is_empty() && appdir.as_deref().is_none_or(|a| !d.starts_with(a)))
        .map(|d| Path::new(d).join(name))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Everything the checks and fixes work on.
pub struct Ctx<'a> {
    pub home: &'a Path,
    pub game: &'a GameInstall,
    /// Per-game folder for fix records, backups and logs.
    pub state_dir: PathBuf,
    pub probe: Probe,
}

// ---- Visual C++ runtime ------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Vc {
    Missing,
    /// Wine's own msvcp140: reports a high version number but isn't the real
    /// runtime, so CET and RED4ext can still fail on it.
    WineBuiltin,
    Native { version: (u16, u16), overridden: bool },
}

impl Vc {
    pub fn ok(&self) -> bool {
        matches!(self, Vc::Native { version, overridden: true } if *version >= MIN_VC_RUNTIME)
    }
}

/// Wine marks its own PE DLLs with this string right after the DOS header.
fn is_wine_builtin(dll: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 0x50];
    std::fs::File::open(dll).and_then(|mut f| f.read_exact(&mut buf)).is_ok() && &buf[0x40..0x50] == b"Wine builtin DLL"
}

pub fn vc_state(prefix: &Path) -> Vc {
    let dll = prefix.join("drive_c/windows/system32/msvcp140.dll");
    if !dll.is_file() {
        return Vc::Missing;
    }
    if is_wine_builtin(&dll) {
        return Vc::WineBuiltin;
    }
    let version = game::exe_versions(&dll)
        .ok()
        .and_then(|(v, _)| v)
        .and_then(|v| {
            let mut p = v.split('.').map(|x| x.parse::<u16>().ok());
            Some((p.next()??, p.next()??))
        })
        .unwrap_or((0, 0));
    let reg = std::fs::read_to_string(prefix.join("user.reg")).unwrap_or_default();
    let overridden = read_overrides(&reg).get("msvcp140").is_some_and(|v| v.starts_with('n'));
    Vc::Native { version, overridden }
}

// ---- Wine registry DLL overrides ---------------------------------------------

const OVERRIDES_SECTION: &str = "[Software\\\\Wine\\\\DllOverrides]";

fn reg_entry(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('"')?;
    let (name, value) = rest.split_once("\"=")?;
    let value = value.trim().trim_matches('"').to_string();
    Some((name.trim_start_matches('*').to_ascii_lowercase(), value))
}

/// `HKCU\Software\Wine\DllOverrides` from a prefix's `user.reg`, keys lowercased.
pub fn read_overrides(reg: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut inside = false;
    for line in reg.lines() {
        if line.starts_with('[') {
            inside = line.starts_with(OVERRIDES_SECTION);
            continue;
        }
        if inside && let Some((k, v)) = reg_entry(line) {
            out.insert(k, v);
        }
    }
    out
}

/// Set (`Some`) or remove (`None`) DLL overrides in `user.reg` text.
pub fn write_overrides(reg: &str, set: &[(String, Option<String>)]) -> String {
    let mut lines: Vec<String> = reg.lines().map(String::from).collect();
    let start = match lines.iter().position(|l| l.starts_with(OVERRIDES_SECTION)) {
        Some(i) => i,
        None => {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(String::new());
            }
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            lines.push(format!("{OVERRIDES_SECTION} {now}"));
            lines.len() - 1
        }
    };
    for (name, value) in set {
        let end = lines[start + 1..].iter().position(|l| l.starts_with('[')).map(|i| start + 1 + i).unwrap_or(lines.len());
        let existing = (start + 1..end).find(|&i| reg_entry(&lines[i]).is_some_and(|(k, _)| k.eq_ignore_ascii_case(name)));
        match (existing, value) {
            (Some(i), Some(v)) => lines[i] = format!("\"{name}\"=\"{v}\""),
            (Some(i), None) => {
                lines.remove(i);
            }
            (None, Some(v)) => {
                // After the section's last entry, before the blank separator.
                let mut at = end;
                while at > start + 1 && lines[at - 1].trim().is_empty() {
                    at -= 1;
                }
                lines.insert(at, format!("\"{name}\"=\"{v}\""));
            }
            (None, None) => {}
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

// ---- Steam launch options ----------------------------------------------------

/// DLLs Wine must load from the game folder before its own: `winmm` and
/// `version` for CET and RED4ext, ReShade's DLL and `d3dcompiler_47` (the
/// shader compiler ReShade calls) when ReShade is in the game, and the name
/// OptiScaler loads as.
pub fn override_dlls(g: &GameInstall) -> Vec<&'static str> {
    let mut v = Vec::new();
    if game::needs_overrides(g) {
        v.extend(["winmm", "version"]);
    }
    v.extend(reshade_dlls(&g.path));
    v.extend(optiscaler_dlls(&g.path));
    v
}

/// OptiScaler's override entry, or none when OptiScaler isn't in the game.
pub fn optiscaler_dlls(game_dir: &Path) -> Vec<&'static str> {
    crate::optiscaler::installed_proxy(game_dir).map(|d| d.trim_end_matches(".dll")).into_iter().collect()
}

/// ReShade's override entries, or none when ReShade isn't in the game.
pub fn reshade_dlls(game_dir: &Path) -> Vec<&'static str> {
    match crate::reshade::installed_dll(game_dir) {
        Some(dll) => vec![dll.trim_end_matches(".dll"), "d3dcompiler_47"],
        None => Vec::new(),
    }
}

/// `winmm,version=n,b`: native first, then Wine's builtin.
pub fn override_value(dlls: &[&str]) -> String {
    format!("{}=n,b", dlls.join(","))
}

/// The WINEDLLOVERRIDES value in launch options, with the byte range of the
/// whole `WINEDLLOVERRIDES=…` assignment.
fn find_overrides(s: &str) -> Option<(String, usize, usize)> {
    let key = "WINEDLLOVERRIDES=";
    let i = s.to_ascii_uppercase().find(key)?;
    let vstart = i + key.len();
    Some(if s[vstart..].starts_with('"') {
        let close = s[vstart + 1..].find('"').map(|j| vstart + 1 + j).unwrap_or(s.len());
        (s[vstart + 1..close].to_string(), i, (close + 1).min(s.len()))
    } else {
        let end = s[vstart..].find(char::is_whitespace).map(|j| vstart + j).unwrap_or(s.len());
        (s[vstart..end].to_string(), i, end)
    })
}

fn dll_key(d: &str) -> String {
    d.trim().trim_end_matches(".dll").to_ascii_lowercase()
}

/// Whether the launch options load every one of `dlls` native first.
pub fn has_overrides(opts: &str, dlls: &[&str]) -> bool {
    let Some((value, ..)) = find_overrides(opts) else { return dlls.is_empty() };
    let mut modes = HashMap::new();
    for entry in value.split(';') {
        let (names, mode) = entry.split_once('=').unwrap_or((entry, ""));
        for n in names.split(',') {
            modes.insert(dll_key(n), mode.trim().to_ascii_lowercase());
        }
    }
    dlls.iter().all(|d| modes.get(&dll_key(d)).is_some_and(|m| m.starts_with('n')))
}

/// Game argument that makes the game deploy and load REDmod mods.
pub const MODDED: &str = "-modded";
/// Launcher argument that starts the game without CD Projekt's REDprelauncher
/// window (wiki: "Additional launch parameters for use from game launchers").
pub const LAUNCHER_SKIP: &str = "--launcher-skip";

fn has_arg(opts: &str, arg: &str) -> bool {
    opts.split_whitespace().any(|o| o.eq_ignore_ascii_case(arg))
}

pub fn has_modded(opts: &str) -> bool {
    has_arg(opts, MODDED)
}

pub fn has_launcher_skip(opts: &str) -> bool {
    has_arg(opts, LAUNCHER_SKIP)
}

/// `arg` right after `%command%` (or at the end when there is none), unless
/// it is already there.
fn add_arg(s: &str, arg: &str) -> String {
    if has_arg(s, arg) {
        return s.to_string();
    }
    match s.find("%command%") {
        Some(i) => format!("{} {arg}{}", &s[..i + "%command%".len()], &s[i + "%command%".len()..]),
        None => format!("{s} {arg}").trim().to_string(),
    }
}

/// The launch options without `arg`; `%command%` alone is the same as empty.
fn remove_arg(s: &str, arg: &str) -> String {
    let s = s.split(' ').filter(|t| !t.eq_ignore_ascii_case(arg)).collect::<Vec<_>>().join(" ");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s == "%command%" { String::new() } else { s }
}

/// The user's launch options with what mods need added, keeping everything
/// else (other variables, other DLL overrides, game arguments). Always skips
/// the REDprelauncher: it opens a window before the game (full screen under
/// gamescope), and its one setting mods need, Enable mods, is `-modded`,
/// which the app sets itself (see [`set_redmod`]).
pub fn merge_launch_options(existing: &str, dlls: &[&str], need_modded: bool) -> String {
    let mut s = existing.trim().to_string();
    if !has_overrides(&s, dlls) {
        let ours = override_value(dlls);
        if let Some((old, i, vend)) = find_overrides(&s) {
            let wanted: Vec<String> = dlls.iter().map(|d| dll_key(d)).collect();
            let mut parts = vec![ours];
            for entry in old.split(';').filter(|e| !e.is_empty()) {
                let (names, mode) = entry.split_once('=').unwrap_or((entry, ""));
                let kept: Vec<&str> = names
                    .split(',')
                    .filter(|d| {
                        let d = dll_key(d);
                        !d.is_empty() && !wanted.contains(&d)
                    })
                    .collect();
                if !kept.is_empty() {
                    parts.push(if mode.is_empty() { kept.join(",") } else { format!("{}={mode}", kept.join(",")) });
                }
            }
            s = format!("{}WINEDLLOVERRIDES=\"{}\"{}", &s[..i], parts.join(";"), &s[vend..]);
        } else if s.contains("%command%") {
            s = format!("WINEDLLOVERRIDES=\"{ours}\" {s}");
        } else {
            // Bare options are game arguments; they go after %command%.
            s = format!("WINEDLLOVERRIDES=\"{ours}\" %command% {s}").trim().to_string();
        }
    }
    if !has_launcher_skip(&s) {
        if !s.contains("%command%") {
            s = format!("%command% {s}").trim().to_string();
        }
        s = add_arg(&s, LAUNCHER_SKIP);
    }
    if need_modded {
        s = add_arg(&s, MODDED);
    }
    s.trim().to_string()
}

const LAUNCH_PATH: &[&str] = &["UserLocalConfigStore", "Software", "Valve", "Steam", "apps", STEAM_APP_ID, "LaunchOptions"];

fn launch_in(src: &str) -> Option<String> {
    let mut v = &vdf::parse(src);
    for k in LAUNCH_PATH {
        v = v.get(k)?;
    }
    v.as_str().map(String::from)
}

/// Every Steam account's `localconfig.vdf` that has an apps list to edit.
fn localconfigs(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in game::steam_roots(home) {
        let Ok(users) = std::fs::read_dir(root.join("userdata")) else { continue };
        for u in users.flatten() {
            let p = u.path().join("config/localconfig.vdf");
            if std::fs::read_to_string(&p).is_ok_and(|s| vdf::set_value(&s, LAUNCH_PATH, "", 2).is_some()) {
                out.push(p);
            }
        }
    }
    out
}

/// Replace a file through a temp file next to it, keeping its permissions.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("cpmx-tmp");
    std::fs::write(&tmp, content)?;
    if let Ok(m) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, m.permissions());
    }
    std::fs::rename(&tmp, path)?;
    activity::record_path(Kind::Setup, "Rewrote", path);
    Ok(())
}

// ---- Folders that differ only in case ------------------------------------------

/// Folder names the game itself uses; when two spellings exist, these win.
const CANONICAL: &[&str] = &[
    "archive", "pc", "mod", "bin", "x64", "plugins", "cyber_engine_tweaks", "mods", "r6", "scripts", "tweaks", "cache", "red4ext",
    "engine", "tools", "config",
];

/// Game-relative folders that exist under several spellings. Wine sees only
/// one of them, so mods in the others don't load.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CaseDup {
    pub keep: String,
    pub others: Vec<String>,
}

fn count_files(dir: &Path) -> usize {
    walkdir::WalkDir::new(dir).into_iter().flatten().filter(|e| e.file_type().is_file()).count()
}

pub fn case_duplicates(game_dir: &Path) -> Vec<CaseDup> {
    let mut out = Vec::new();
    scan_dups(game_dir, game_dir, 0, &mut out);
    out
}

fn scan_dups(game_dir: &Path, dir: &Path, depth: usize, out: &mut Vec<CaseDup>) {
    if depth > 10 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for e in rd.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        // At the top only the folders mods go into; the rest is the game's.
        if depth == 0 && !KNOWN_ROOTS.iter().any(|r| r.eq_ignore_ascii_case(&name)) {
            continue;
        }
        groups.entry(name.to_lowercase()).or_default().push(e.path());
    }
    let mut keys: Vec<_> = groups.keys().cloned().collect();
    keys.sort();
    for k in keys {
        let mut members = groups.remove(&k).unwrap_or_default();
        members.sort();
        let keep = if members.len() > 1 {
            let name = |p: &PathBuf| p.file_name().unwrap_or_default().to_string_lossy().to_string();
            let keep = members
                .iter()
                .position(|p| CANONICAL.contains(&name(p).as_str()))
                .unwrap_or_else(|| {
                    let counts: Vec<usize> = members.iter().map(|m| count_files(m)).collect();
                    let max = counts.iter().copied().max().unwrap_or(0);
                    counts.iter().position(|&c| c == max).unwrap_or(0)
                });
            let rel = |p: &PathBuf| p.strip_prefix(game_dir).unwrap_or(p).to_string_lossy().to_string();
            out.push(CaseDup {
                keep: rel(&members[keep]),
                others: members.iter().enumerate().filter(|(i, _)| *i != keep).map(|(_, p)| rel(p)).collect(),
            });
            members.swap_remove(keep)
        } else {
            members.swap_remove(0)
        };
        scan_dups(game_dir, &keep, depth + 1, out);
    }
}

/// Move everything from the other spellings into the kept folder. Files that
/// already exist there are left where they are and returned as conflicts.
type Moves = Vec<(String, String)>;

fn merge_dups(game_dir: &Path, dups: &[CaseDup]) -> Result<(Moves, Vec<String>)> {
    let mut moves = Vec::new();
    let mut conflicts = Vec::new();
    for d in dups {
        let keep = game_dir.join(&d.keep);
        for other in &d.others {
            let other_abs = game_dir.join(other);
            let files: Vec<PathBuf> = walkdir::WalkDir::new(&other_abs)
                .into_iter()
                .flatten()
                .filter(|e| !e.file_type().is_dir())
                .map(|e| e.into_path())
                .collect();
            for f in files {
                let rel = f.strip_prefix(&other_abs).unwrap_or(&f).to_string_lossy().to_string();
                let dst = resolve_ci(&keep, &rel);
                let from = f.strip_prefix(game_dir).unwrap_or(&f).to_string_lossy().to_string();
                if dst.symlink_metadata().is_ok() || !dst.starts_with(&keep) {
                    conflicts.push(from);
                    continue;
                }
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&f, &dst)?;
                activity::record_path(Kind::Move, format!("Moved {} to", f.display()), &dst);
                moves.push((from, dst.strip_prefix(game_dir).unwrap_or(&dst).to_string_lossy().to_string()));
            }
            remove_empty_dirs(&other_abs);
        }
    }
    Ok((moves, conflicts))
}

fn remove_empty_dirs(dir: &Path) {
    let dirs: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .contents_first(true)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();
    for d in dirs {
        let _ = std::fs::remove_dir(d); // fails, as it should, when not empty
    }
}

// ---- Records of applied fixes ------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    applied_at: u64,
    /// localconfig.vdf files and their launch options before.
    #[serde(default)]
    launch: Vec<(PathBuf, Option<String>)>,
    /// DLL overrides before.
    #[serde(default)]
    overrides: Vec<(String, Option<String>)>,
    /// Game-relative (from, to) file moves.
    #[serde(default)]
    moves: Moves,
    /// Runtime DLLs that existed before, per system dir (copies in the backup).
    #[serde(default)]
    vc_files: Vec<(String, Vec<String>)>,
}

fn record_path(ctx: &Ctx, id: &str) -> PathBuf {
    ctx.state_dir.join(format!("{id}.json"))
}

fn load_record(ctx: &Ctx, id: &str) -> Option<Record> {
    serde_json::from_str(&std::fs::read_to_string(record_path(ctx, id)).ok()?).ok()
}

fn save_record(ctx: &Ctx, id: &str, r: &Record) -> Result<()> {
    std::fs::create_dir_all(&ctx.state_dir)?;
    std::fs::write(record_path(ctx, id), serde_json::to_vec_pretty(r)?)?;
    Ok(())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---- Checks --------------------------------------------------------------------

const CLOSE_GAME: &str = "Cyberpunk 2077 is running. Quit the game first.";
const CLOSE_STEAM: &str =
    "Steam is running. Quit Steam completely (Steam › Exit), then try again: Steam rewrites this setting when it quits.";

fn action(label: &str, confirm: String, blocked: Option<&str>) -> Option<Action> {
    Some(Action { label: label.into(), confirm, blocked: blocked.map(String::from) })
}

fn undo_action(ctx: &Ctx, id: &str, what: &str, blocked: Option<&str>) -> Option<Action> {
    load_record(ctx, id).map(|_| Action {
        label: "Undo".into(),
        confirm: format!("Undo the earlier fix and put back {what} as it was before?"),
        blocked: blocked.map(String::from),
    })
}

/// How the runtime would be installed for this game, or why it can't be.
fn vc_installer(ctx: &Ctx) -> std::result::Result<(Tricks, Option<PathBuf>), String> {
    let p = &ctx.probe;
    let winetricks_with = |wine: Option<PathBuf>| -> std::result::Result<(Tricks, Option<PathBuf>), String> {
        match (&p.winetricks, wine) {
            (Some(w), Some(wine)) if p.cabextract => Ok((Tricks::Winetricks(w.clone()), Some(wine))),
            (Some(_), Some(_)) => Err("winetricks needs cabextract; install your distribution's cabextract package.".into()),
            _ => Err(String::new()),
        }
    };
    match ctx.game.store {
        Store::Steam => {
            if let Some(t) = &p.protontricks {
                return Ok((t.clone(), None));
            }
            let wine = ctx.game.proton_prefix.as_deref().and_then(steam_proton_wine);
            winetricks_with(wine).map_err(|e| {
                if e.is_empty() {
                    "Install protontricks first (Flathub: flatpak install flathub com.github.Matoking.protontricks, \
                     or your distribution's protontricks package), then click again."
                        .into()
                } else {
                    e
                }
            })
        }
        _ => {
            let wine = game::heroic_game(ctx.home, &ctx.game.path).and_then(|h| h.wine);
            winetricks_with(wine).map_err(|e| {
                if e.is_empty() {
                    "Install winetricks and cabextract from your distribution, then click again. \
                     The Wine build is read from Heroic's settings for this game."
                        .into()
                } else {
                    e
                }
            })
        }
    }
}

/// The Proton build that last ran this prefix (`config_info` names its fonts dir).
fn steam_proton_dir(prefix: &Path) -> Option<PathBuf> {
    let info = std::fs::read_to_string(prefix.parent()?.join("config_info")).ok()?;
    let fonts = info.lines().nth(1)?;
    let dir = ["/files/", "/dist/"].iter().find_map(|m| fonts.find(m).map(|i| &fonts[..i]))?;
    Some(PathBuf::from(dir))
}

fn steam_proton_wine(prefix: &Path) -> Option<PathBuf> {
    game::proton_wine(&steam_proton_dir(prefix)?)
}

/// The Proton/Wine build that runs the game: Steam's last one for the
/// prefix, or the one set in Heroic.
fn runner(ctx: &Ctx) -> Option<hdr::Runner> {
    let path = match ctx.game.store {
        Store::Steam => steam_proton_dir(ctx.game.proton_prefix.as_deref()?)?,
        _ => game::heroic_game(ctx.home, &ctx.game.path)?.wine?,
    };
    Some(hdr::runner(&path))
}

/// On Windows the game runs natively: there's no prefix, no overrides and no
/// case-sensitive filesystem, so there is nothing to check.
pub fn checks(ctx: &Ctx) -> Vec<Check> {
    if cfg!(windows) {
        return Vec::new();
    }
    let g = ctx.game;
    let needs = game::needs_overrides(g);
    let reshade = reshade_dlls(&g.path);
    let dlls = override_dlls(g);
    let redmods = game::has_redmods(&g.path);
    let running = ctx.probe.game_running.then_some(CLOSE_GAME);
    let mut out = Vec::new();
    // Who needs the overrides, for the explanations.
    let mut who: Vec<&str> = Vec::new();
    if needs {
        who.extend(["CET", "RED4ext"]);
    }
    if !reshade.is_empty() {
        who.push("ReShade");
    }
    if !optiscaler_dlls(&g.path).is_empty() {
        who.push("OptiScaler");
    }
    let for_whom = match who.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.to_string(),
        None => "mods".to_string(),
    };

    // Visual C++ runtime.
    let vc_record = load_record(ctx, FIX_VC).is_some();
    if needs || vc_record {
        match &g.proton_prefix {
            None if g.store == Store::Steam => out.push(Check {
                alt: None,
                alt_id: None,
                id: FIX_VC.into(),
                title: "Visual C++ runtime".into(),
                state: State::Problem,
                detail: "Proton hasn't created this game's prefix yet. Start the game once from Steam, quit, then rescan.".into(),
                fix: None,
                undo: None,
                copy: None,
            }),
            None => {}
            Some(prefix) => {
                let vc = vc_state(prefix);
                let detail = match &vc {
                    _ if vc.ok() => "Visual C++ 2015-2022 runtime is installed in the prefix.".to_string(),
                    Vc::Missing => "The prefix has no Visual C++ runtime. CET and RED4ext fail to start without it (error 998 or a missing DLL).".into(),
                    Vc::WineBuiltin => "The prefix only has Wine's stand-in for the Visual C++ runtime. CET and RED4ext need Microsoft's (error 998 at startup).".into(),
                    Vc::Native { version, overridden: false } if *version >= MIN_VC_RUNTIME => {
                        "Microsoft's Visual C++ runtime is in the prefix, but Wine is set to use its own copy instead, so CET and RED4ext can fail with error 998.".into()
                    }
                    Vc::Native { version, .. } => format!(
                        "The Visual C++ runtime in the prefix is {}.{}, older than CET and RED4ext need ({}.{} or newer); they fail at startup with error 998.",
                        version.0, version.1, MIN_VC_RUNTIME.0, MIN_VC_RUNTIME.1
                    ),
                };
                let fix = (!vc.ok()).then(|| {
                    let (blocked, how) = match vc_installer(ctx) {
                        Ok((t, _)) => (running.map(String::from), match t {
                            Tricks::Protontricks(_) | Tricks::ProtontricksFlatpak => "protontricks",
                            Tricks::Winetricks(_) => "winetricks",
                        }),
                        Err(e) => (Some(running.map(String::from).unwrap_or(e)), "protontricks"),
                    };
                    Action {
                        label: "Install vcrun2022".into(),
                        confirm: format!(
                            "Install Microsoft's Visual C++ 2015-2022 runtime (vcrun2022) into the game's prefix with {how}?\n\n\
                             It downloads the installer from Microsoft and can take a few minutes. The runtime files and prefix \
                             registry are backed up first, and Undo puts them back."
                        ),
                        blocked,
                    }
                });
                out.push(Check {
                    alt: None,
                    alt_id: None,
                    id: FIX_VC.into(),
                    title: "Visual C++ runtime".into(),
                    state: if vc.ok() { State::Ok } else { State::Problem },
                    detail,
                    fix,
                    undo: undo_action(ctx, FIX_VC, "the prefix's runtime files and registry", running),
                    copy: None,
                });
            }
        }
    }

    // The shader compiler ReShade calls.
    if let Some(prefix) = &g.proton_prefix
        && (!reshade.is_empty() || load_record(ctx, FIX_D3DCOMPILER).is_some())
    {
        let ok = compiler_ok(prefix);
        let fix = (!ok).then(|| {
            let (blocked, how) = match vc_installer(ctx) {
                Ok((Tricks::Winetricks(_), _)) => (running.map(String::from), "winetricks"),
                Ok(_) => (running.map(String::from), "protontricks"),
                Err(e) => (Some(running.map(String::from).unwrap_or(e)), "protontricks"),
            };
            Action {
                label: "Install d3dcompiler_47".into(),
                confirm: format!(
                    "Install Microsoft's shader compiler (d3dcompiler_47) into the game's prefix with {how}?\n\n\
                     winetricks takes the DLL from Mozilla's Firefox installer. The compiler files and prefix registry \
                     are backed up first, and Undo puts them back."
                ),
                blocked,
            }
        });
        out.push(Check {
            alt: None,
            alt_id: None,
            id: FIX_D3DCOMPILER.into(),
            title: "Shader compiler for ReShade".into(),
            state: if ok { State::Ok } else { State::Problem },
            detail: if ok {
                "Microsoft's d3dcompiler_47.dll is in the prefix.".into()
            } else {
                "The prefix has only Wine's d3dcompiler_47.dll. Many ReShade effects fail to compile with it \
                 (compile errors in ReShade's overlay)."
                    .into()
            },
            fix,
            undo: undo_action(ctx, FIX_D3DCOMPILER, "the prefix's compiler files and registry", running),
            copy: None,
        });
    }

    // DLL overrides, -modded and skipping the REDprelauncher.
    if g.store == Store::Steam {
        let modded = redmods && redmod_wanted(ctx);
        let cur = g.launch_options.clone().unwrap_or_default();
        let want = merge_launch_options(&cur, &dlls, modded);
        let ok = want == cur.trim();
        let mut missing = Vec::new();
        if !has_overrides(&cur, &dlls) {
            missing.push(format!("WINEDLLOVERRIDES=\"{}\" so {for_whom} load", override_value(&dlls)));
        }
        if modded && !has_modded(&cur) {
            missing.push("-modded so REDmod mods load".into());
        }
        if !has_launcher_skip(&cur) {
            missing.push(format!("{LAUNCHER_SKIP} so the game starts without the REDprelauncher window"));
        }
        let steam_block = if ctx.probe.steam_running { Some(CLOSE_STEAM) } else { None };
        let no_config = localconfigs(ctx.home).is_empty().then_some("Steam has no settings for this game yet. Start it once from Steam, then try again.");
        out.push(Check {
            alt: None,
            alt_id: None,
            id: FIX_LAUNCH.into(),
            title: "Steam launch options".into(),
            state: if ok { State::Ok } else { State::Problem },
            detail: if ok {
                format!("Launch options are set: {cur}")
            } else {
                format!("Steam's launch options need {}.", missing.join(" and "))
            },
            fix: if ok {
                None
            } else {
                action(
                    "Set launch options",
                    format!(
                        "Set Cyberpunk 2077's Steam launch options to:\n\n{want}\n\nNow: {}\n\nUndo puts the old ones back.",
                        if cur.trim().is_empty() { "(empty)" } else { cur.trim() }
                    ),
                    steam_block.or(no_config),
                )
            },
            undo: undo_action(ctx, FIX_LAUNCH, "the previous launch options", steam_block),
            copy: (!ok).then_some(want),
        });
    } else if let Some(prefix) = &g.proton_prefix {
        let rec = load_record(ctx, FIX_OVERRIDES).is_some();
        if !dlls.is_empty() || rec {
            let reg = std::fs::read_to_string(prefix.join("user.reg")).unwrap_or_default();
            let o = read_overrides(&reg);
            let ok = dlls.iter().all(|d| o.get(*d).is_some_and(|v| v.starts_with('n')));
            let names = dlls.join(", ");
            out.push(Check {
                alt: None,
                alt_id: None,
                id: FIX_OVERRIDES.into(),
                title: "DLL overrides".into(),
                state: if ok { State::Ok } else { State::Problem },
                detail: if ok {
                    format!("{names}: native DLLs load first.")
                } else {
                    format!("{for_whom} need Wine to load these DLLs native first: {names}.")
                },
                fix: if ok {
                    None
                } else {
                    action(
                        "Set overrides",
                        format!(
                            "Set {names} to “native, then builtin” in the game's Wine prefix? This works for any launcher, \
                             so no launch options are needed. Undo puts the old settings back."
                        ),
                        running,
                    )
                },
                undo: undo_action(ctx, FIX_OVERRIDES, "the previous DLL overrides", running),
                copy: None,
            });
        }
    }

    // HDR, when a connected display supports it.
    let steam = g.store == Store::Steam;
    if let Some(st) = hdr::status(&ctx.probe.hdr, runner(ctx).as_ref(), steam) {
        let cur = if steam {
            g.launch_options.clone().unwrap_or_default()
        } else {
            let env = game::heroic_game(ctx.home, &g.path).map(|h| h.env).unwrap_or_default();
            env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ")
        };
        let on_alt = st.alt.as_ref().is_some_and(|p| hdr::has_plan(&cur, p));
        let set = on_alt || st.plan.as_ref().is_some_and(|p| hdr::has_plan(&cur, p));
        // A display in HDR mode running the game in SDR loses quality; one
        // the user keeps in SDR is fine.
        let ok = (set && st.missing.is_empty()) || st.sdr;
        let mut detail = format!("{}.", st.found.join("; "));
        if !st.missing.is_empty() {
            detail.push_str(&format!(" Missing: {}.", st.missing.join("; ")));
        }
        let want = st.plan.as_ref().map(|p| hdr::merge(&if on_alt { hdr::strip(&cur) } else { cur.clone() }, p));
        if steam && st.plan.as_ref().is_some_and(|p| p.gamescope.is_some()) {
            detail.push_str(" Without gamescope: GE-Proton 10 or newer (Steam › Properties › Compatibility) outputs HDR through Wine's Wayland driver.");
        }
        match &st.plan {
            Some(p) if !set && steam => detail.push_str(&format!(" Launch options lack {}.", p.line())),
            Some(p) if !set => detail.push_str(&format!(
                " Heroic › game settings › Advanced › Environment variables lacks {}.",
                p.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")
            )),
            _ => {}
        }
        if set {
            detail.push_str(" In game: Settings › Video › HDR Mode › HDR10 PQ.");
        }
        let steam_block = if ctx.probe.steam_running { Some(CLOSE_STEAM) } else { None };
        let no_config = localconfigs(ctx.home).is_empty().then_some("Steam has no settings for this game yet. Start it once from Steam, then try again.");
        let confirm = |w: &str, method: &str| {
            format!(
                "Set Cyberpunk 2077's Steam launch options to ({method}):\n\n{w}\n\nNow: {}\n\nUndo removes the HDR settings again.",
                if cur.trim().is_empty() { "(empty)" } else { cur.trim() }
            )
        };
        // Wayland is offered first; gamescope stays available when installed.
        let (alt, alt_id) = match (&st.alt, &want) {
            (Some(_), Some(w)) if steam && on_alt => (action("Use Wayland instead", confirm(w, "Wine Wayland driver"), steam_block.or(no_config)), Some(FIX_HDR_WAYLAND)),
            (Some(a), _) if steam => (
                action(
                    if set { "Use gamescope instead" } else { "Set HDR with gamescope" },
                    confirm(&hdr::merge(&hdr::strip(&cur), a), "gamescope"),
                    steam_block.or(no_config),
                ),
                Some(FIX_HDR_GAMESCOPE),
            ),
            _ => (None, None),
        };
        out.push(Check {
            alt,
            alt_id: alt_id.map(String::from),
            id: FIX_HDR.into(),
            title: if st.sdr && !set { "HDR (desktop in SDR mode)".into() } else { "HDR".into() },
            state: if ok { State::Ok } else { State::Problem },
            detail,
            fix: match &want {
                Some(want) if steam && !set => action(
                    "Set HDR launch options",
                    format!(
                        "Set Cyberpunk 2077's Steam launch options to:\n\n{want}\n\nNow: {}\n\nUndo removes the HDR settings again.",
                        if cur.trim().is_empty() { "(empty)" } else { cur.trim() }
                    ),
                    steam_block.or(no_config),
                ),
                _ => None,
            },
            undo: undo_action(ctx, FIX_HDR, "the launch options without the HDR settings", steam_block),
            copy: match (&st.plan, want) {
                (Some(_), Some(w)) if steam && !set => Some(w),
                (Some(p), _) if !set => Some(p.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ")),
                _ => None,
            },
        });
    }

    // FSR 4.1.1 shader overrides for vkd3d-proton, once picked on the
    // OptiScaler page.
    if let Some(c) = fsr4_check(ctx) {
        out.push(c);
    }

    // Folders that exist under two spellings.
    let dups = case_duplicates(&g.path);
    let rec = load_record(ctx, FIX_CASE).is_some();
    if !dups.is_empty() || rec {
        let list: Vec<String> = dups.iter().map(|d| format!("{} ({})", d.keep, d.others.join(", "))).collect();
        out.push(Check {
            alt: None,
            alt_id: None,
            id: FIX_CASE.into(),
            title: "Folder names".into(),
            state: if dups.is_empty() { State::Ok } else { State::Problem },
            detail: if dups.is_empty() {
                "Every mod folder has a single spelling.".into()
            } else {
                format!(
                    "These folders exist more than once with different capitalisation, and the game only reads one of them: {}",
                    list.join("; ")
                )
            },
            fix: (!dups.is_empty()).then(|| Action {
                label: "Merge folders".into(),
                confirm: format!(
                    "Move the files from the other spellings into the folder the game reads?\n\n{}\n\nFiles that exist in both are left in place. Undo moves everything back.",
                    list.join("\n")
                ),
                blocked: running.map(String::from),
            }),
            undo: undo_action(ctx, FIX_CASE, "the moved files", running),
            copy: None,
        });
    }
    out
}

/// What ReShade needs from Proton, for the ReShade card.
#[derive(Debug, Clone, Serialize)]
pub struct ReShadeLaunch {
    /// Steam: the whole launch options line to use. Other launchers: the
    /// environment variable to set.
    pub line: String,
    pub steam: bool,
    /// ReShade's DLL overrides are set (launch options, or the prefix's
    /// registry for other launchers).
    pub ok: bool,
    /// Microsoft's shader compiler is in the prefix; `None` without a prefix.
    pub compiler: Option<bool>,
}

/// `None` on Windows and when no ReShade DLL is in the game folder.
pub fn reshade_launch(g: &GameInstall) -> Option<ReShadeLaunch> {
    if cfg!(windows) {
        return None;
    }
    let ours = reshade_dlls(&g.path);
    if ours.is_empty() {
        return None;
    }
    let dlls = override_dlls(g);
    let compiler = g.proton_prefix.as_deref().map(compiler_ok);
    Some(if g.store == Store::Steam {
        let cur = g.launch_options.clone().unwrap_or_default();
        ReShadeLaunch { line: merge_launch_options(&cur, &dlls, has_modded(&cur)), steam: true, ok: has_overrides(&cur, &ours), compiler }
    } else {
        let reg = g.proton_prefix.as_ref().and_then(|p| std::fs::read_to_string(p.join("user.reg")).ok()).unwrap_or_default();
        let o = read_overrides(&reg);
        let ok = ours.iter().all(|d| o.get(*d).is_some_and(|v| v.starts_with('n')));
        ReShadeLaunch { line: format!("WINEDLLOVERRIDES=\"{}\"", override_value(&dlls)), steam: false, ok, compiler }
    })
}

// ---- FSR 4.1.1 shader overrides ------------------------------------------------

/// `KEY=value` (quoted or bare) in launch options: the value and the byte
/// range of the whole assignment.
fn find_env(s: &str, key: &str) -> Option<(String, usize, usize)> {
    let pat = format!("{key}=");
    let mut from = 0;
    let i = loop {
        let i = from + s[from..].find(&pat)?;
        if i == 0 || s[..i].ends_with(char::is_whitespace) {
            break i;
        }
        from = i + pat.len();
    };
    let vstart = i + pat.len();
    Some(match s[vstart..].chars().next() {
        Some(q @ ('\'' | '"')) => {
            let close = s[vstart + 1..].find(q).map(|j| vstart + 1 + j).unwrap_or(s.len());
            (s[vstart + 1..close].to_string(), i, (close + 1).min(s.len()))
        }
        _ => {
            let end = s[vstart..].find(char::is_whitespace).map(|j| vstart + j).unwrap_or(s.len());
            (s[vstart..end].to_string(), i, end)
        }
    })
}

/// Launch options with `key` set to `value` (single-quoted, in front of
/// `%command%`), or with `key` taken out for `None`. Everything else stays.
pub fn merge_env(opts: &str, key: &str, value: Option<&str>) -> String {
    let mut s = opts.trim().to_string();
    while let Some((_, i, end)) = find_env(&s, key) {
        s = format!("{} {}", s[..i].trim_end(), s[end..].trim_start()).trim().to_string();
    }
    match value {
        Some(v) => {
            if !s.contains("%command%") {
                // Bare options are game arguments; they go after %command%.
                s = format!("%command% {s}").trim().to_string();
            }
            format!("{key}='{v}' {s}")
        }
        None if s == "%command%" => String::new(),
        None => s,
    }
}

fn fsr4_check(ctx: &Ctx) -> Option<Check> {
    use crate::optiscaler::{OVERRIDE_VAR, Variant, load_choice, override_value};
    let g = ctx.game;
    let choice = load_choice(&ctx.state_dir);
    if choice.is_none() && load_record(ctx, FIX_FSR4).is_none() {
        return None;
    }
    let steam = g.store == Store::Steam;
    let cur = if steam {
        g.launch_options.clone().unwrap_or_default()
    } else {
        let env = game::heroic_game(ctx.home, &g.path).map(|h| h.env).unwrap_or_default();
        env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ")
    };
    let now = find_env(&cur, OVERRIDE_VAR).map(|(v, ..)| v);
    let steam_block = if ctx.probe.steam_running { Some(CLOSE_STEAM) } else { None };
    let no_config = localconfigs(ctx.home).is_empty().then_some("Steam has no settings for this game yet. Start it once from Steam, then try again.");
    let shown = |s: &str| if s.trim().is_empty() { "(empty)".to_string() } else { s.trim().to_string() };
    let mut check = Check {
        id: FIX_FSR4.into(),
        title: "FSR 4 shader overrides".into(),
        state: State::Ok,
        detail: String::new(),
        fix: None,
        undo: undo_action(ctx, FIX_FSR4, "the launch options without the shader overrides", steam_block),
        copy: None,
        alt: None,
        alt_id: None,
    };
    match choice {
        Some(c) => {
            let name = if c.variant == Variant::Lossy { "lossy" } else { "exact" };
            let Some(want) = override_value(&c.folder) else {
                check.state = State::Problem;
                check.detail = format!("{} can't be passed in a launch option (quote or not an absolute path).", c.folder.display());
                return Some(check);
            };
            if !c.folder.is_dir() {
                check.state = State::Problem;
                check.detail = format!("The {name} shader overrides are missing from {}. Download them again on the OptiScaler page.", c.folder.display());
                return Some(check);
            }
            if now.as_deref() == Some(want.as_str()) {
                check.detail = format!("{OVERRIDE_VAR} loads the {name} shader overrides. They apply only while the game runs AMD's {UPSCALER} {}.", crate::optiscaler::FSR_VERSION);
                return Some(check);
            }
            check.state = State::Problem;
            let line = merge_env(&cur, OVERRIDE_VAR, Some(&want));
            if steam {
                check.detail = format!("Launch options lack {OVERRIDE_VAR}='{want}' ({name} shader overrides).");
                check.fix = action(
                    "Set launch option",
                    format!("Set Cyberpunk 2077's Steam launch options to:\n\n{line}\n\nNow: {}\n\nUndo removes the shader overrides again.", shown(&cur)),
                    steam_block.or(no_config),
                );
                check.copy = Some(line);
            } else {
                check.detail = format!("The launcher's environment variables for the game lack {OVERRIDE_VAR}={want} ({name} shader overrides).");
                check.copy = Some(format!("{OVERRIDE_VAR}={want}"));
            }
        }
        None => {
            let Some(old) = now else {
                check.detail = "Shader overrides are off.".into();
                return Some(check);
            };
            check.state = State::Problem;
            check.detail = format!("Shader overrides are off, but the launch options still set {OVERRIDE_VAR}={old}.");
            if steam {
                let line = merge_env(&cur, OVERRIDE_VAR, None);
                check.fix = action(
                    "Remove launch option",
                    format!("Set Cyberpunk 2077's Steam launch options to:\n\n{}\n\nNow: {}", shown(&line), shown(&cur)),
                    steam_block.or(no_config),
                );
            }
        }
    }
    Some(check)
}

const UPSCALER: &str = crate::optiscaler::UPSCALER_DLL;

/// Write the shader-override launch option the FSR 4 check asks for.
fn write_fsr4(ctx: &Ctx) -> Result<String> {
    use crate::optiscaler::{OVERRIDE_VAR, load_choice, override_value};
    let want = load_choice(&ctx.state_dir).and_then(|c| override_value(&c.folder));
    let mut rec = load_record(ctx, FIX_FSR4).unwrap_or_default();
    let mut set = String::new();
    for cfg in localconfigs(ctx.home) {
        let src = std::fs::read_to_string(&cfg)?;
        let before = launch_in(&src);
        let line = merge_env(before.as_deref().unwrap_or(""), OVERRIDE_VAR, want.as_deref());
        if before.as_deref().unwrap_or("") == line {
            continue;
        }
        let new = vdf::set_value(&src, LAUNCH_PATH, &line, 2).ok_or_else(|| Error::Other(format!("could not edit {}", cfg.display())))?;
        if !rec.launch.iter().any(|(p, _)| p == &cfg) {
            rec.launch.push((cfg.clone(), before));
        }
        write_atomic(&cfg, &new)?;
        set = line;
    }
    if want.is_none() {
        // Nothing of ours left to undo.
        let _ = std::fs::remove_file(record_path(ctx, FIX_FSR4));
        return Ok(format!("Shader overrides removed from the launch options: {}", if set.is_empty() { "(empty)" } else { &set }));
    }
    rec.applied_at = now();
    save_record(ctx, FIX_FSR4, &rec)?;
    Ok(format!("Launch options set: {set}"))
}

// ---- REDmod on/off -------------------------------------------------------------

/// The user's choice for REDmod mods, kept so Set launch options doesn't
/// add `-modded` back after they turned REDmod off.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RedmodPref {
    enabled: bool,
}

const REDMOD_PREF: &str = "redmod";

/// REDmod mods should load: on unless the user switched them off.
fn redmod_wanted(ctx: &Ctx) -> bool {
    let p = ctx.state_dir.join(format!("{REDMOD_PREF}.json"));
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str::<RedmodPref>(&s).ok())
        .is_none_or(|p| p.enabled)
}

/// The REDmod switch in the Game panel: replaces the REDprelauncher's
/// Enable mods setting, which only adds `-modded` to the game's arguments.
#[derive(Debug, Clone, Serialize)]
pub struct Redmod {
    /// `-modded` is in Steam's launch options.
    pub enabled: bool,
    /// `mods/<folder>/info.json` folders in the game.
    pub mods: usize,
    /// The switch writes Steam's launch options; elsewhere `copy` says what
    /// to set by hand.
    pub steam: bool,
    pub blocked: Option<String>,
    pub copy: Option<String>,
}

fn redmod_count(game_dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(game_dir.join("mods")) else { return 0 };
    rd.flatten().filter(|e| e.path().join("info.json").is_file()).count()
}

/// `None` when the game has no REDmod tool (`tools/redmod`).
pub fn redmod(ctx: &Ctx) -> Option<Redmod> {
    let g = ctx.game;
    if !game::exists_ci(&g.path, "tools/redmod/bin/redMod.exe") {
        return None;
    }
    let steam = g.store == Store::Steam;
    let cur = g.launch_options.clone().unwrap_or_default();
    let blocked = if !steam {
        None
    } else if ctx.probe.steam_running {
        Some(CLOSE_STEAM.to_string())
    } else if localconfigs(ctx.home).is_empty() {
        Some("Steam has no settings for this game yet. Start it once from Steam, then try again.".into())
    } else {
        None
    };
    Some(Redmod {
        enabled: steam && has_modded(&cur),
        mods: redmod_count(&g.path),
        steam,
        blocked,
        copy: (!steam).then(|| format!("{MODDED} {LAUNCHER_SKIP}")),
    })
}

/// Switch REDmod mods on or off: adds or removes `-modded` in every Steam
/// account's launch options for the game, adds `--launcher-skip`, and keeps
/// the choice. With `-modded` the game deploys changed REDmods itself at
/// start (wiki: Launch parameters, "Triggers REDMod recompilation on start").
pub fn set_redmod(ctx: &Ctx, on: bool) -> Result<String> {
    let state = redmod(ctx).ok_or_else(|| Error::Other("REDmod is not installed (tools/redmod/bin/redMod.exe not found)".into()))?;
    if !state.steam {
        return Err(Error::Other(format!("Launch options are only written for Steam. Add {MODDED} {LAUNCHER_SKIP} to the game arguments in your launcher.")));
    }
    if let Some(b) = state.blocked {
        return Err(Error::Other(b));
    }
    let mut set = None;
    for cfg in localconfigs(ctx.home) {
        let src = std::fs::read_to_string(&cfg)?;
        let before = launch_in(&src).unwrap_or_default();
        let base = match before.trim() {
            "" => "%command%".to_string(),
            b if b.contains("%command%") => b.to_string(),
            b => format!("%command% {b}"),
        };
        let want = add_arg(&base, LAUNCHER_SKIP);
        let want = if on { add_arg(&want, MODDED) } else { remove_arg(&want, MODDED) };
        if want != before {
            let new = vdf::set_value(&src, LAUNCH_PATH, &want, 2).ok_or_else(|| Error::Other(format!("could not edit {}", cfg.display())))?;
            write_atomic(&cfg, &new)?;
        }
        set = Some(want);
    }
    std::fs::create_dir_all(&ctx.state_dir)?;
    std::fs::write(ctx.state_dir.join(format!("{REDMOD_PREF}.json")), serde_json::to_vec(&RedmodPref { enabled: on })?)?;
    let set = set.unwrap_or_default();
    Ok(if on {
        format!("REDmod mods enabled. Launch options: {set}")
    } else {
        format!("REDmod mods disabled. Launch options: {set}")
    })
}

// ---- Applying and undoing ------------------------------------------------------

fn find<'a>(checks: &'a [Check], id: &str) -> Result<&'a Check> {
    checks.iter().find(|c| c.id == id).ok_or_else(|| Error::Other(format!("nothing to do for {id}")))
}

/// Switch HDR between Wine's Wayland driver and gamescope.
fn apply_hdr_choice(ctx: &Ctx, id: &str) -> Result<String> {
    let all = checks(ctx);
    let check = find(&all, FIX_HDR)?;
    let action = check.alt.as_ref().filter(|_| check.alt_id.as_deref() == Some(id)).ok_or_else(|| Error::Other("that HDR method is not available".into()))?;
    if let Some(b) = &action.blocked {
        return Err(Error::Other(b.clone()));
    }
    let st = hdr::status(&ctx.probe.hdr, runner(ctx).as_ref(), true).ok_or_else(|| Error::Other("no HDR display".into()))?;
    let plan = if id == FIX_HDR_GAMESCOPE { st.alt } else { st.plan }.ok_or_else(|| Error::Other("no HDR launch options for this system".into()))?;
    write_hdr(ctx, &plan)
}

/// Replace whatever HDR settings the launch options carry with `plan`. The
/// record keeps each file's options from before the first HDR change.
fn write_hdr(ctx: &Ctx, plan: &hdr::Plan) -> Result<String> {
    let mut rec = load_record(ctx, FIX_HDR).unwrap_or_default();
    let mut set = String::new();
    for cfg in localconfigs(ctx.home) {
        let src = std::fs::read_to_string(&cfg)?;
        let before = launch_in(&src);
        let want = hdr::merge(&hdr::strip(before.as_deref().unwrap_or("")), plan);
        if before.as_deref() == Some(want.as_str()) {
            continue;
        }
        let new = vdf::set_value(&src, LAUNCH_PATH, &want, 2).ok_or_else(|| Error::Other(format!("could not edit {}", cfg.display())))?;
        if !rec.launch.iter().any(|(p, _)| p == &cfg) {
            rec.launch.push((cfg.clone(), before));
        }
        write_atomic(&cfg, &new)?;
        set = want;
    }
    rec.applied_at = now();
    save_record(ctx, FIX_HDR, &rec)?;
    Ok(format!("Launch options set: {set}"))
}

/// Apply one fix after the user confirmed it. Returns a line for the user.
pub fn apply(ctx: &Ctx, id: &str) -> Result<String> {
    if id == FIX_HDR_GAMESCOPE || id == FIX_HDR_WAYLAND {
        return apply_hdr_choice(ctx, id);
    }
    let all = checks(ctx);
    let check = find(&all, id)?;
    let fix = check.fix.as_ref().ok_or_else(|| Error::Other(format!("{} is already fine", check.title)))?;
    if let Some(b) = &fix.blocked {
        return Err(Error::Other(b.clone()));
    }
    let g = ctx.game;
    match id {
        FIX_VC | FIX_D3DCOMPILER => apply_tricks(ctx, id),
        FIX_FSR4 => write_fsr4(ctx),
        FIX_LAUNCH => {
            let dlls = override_dlls(g);
            let redmods = game::has_redmods(&g.path) && redmod_wanted(ctx);
            let mut rec = load_record(ctx, FIX_LAUNCH).unwrap_or_default();
            let first = rec.launch.is_empty();
            let mut set = String::new();
            for cfg in localconfigs(ctx.home) {
                let src = std::fs::read_to_string(&cfg)?;
                let before = launch_in(&src);
                let want = merge_launch_options(before.as_deref().unwrap_or(""), &dlls, redmods);
                if before.as_deref() == Some(want.as_str()) {
                    continue;
                }
                let new = vdf::set_value(&src, LAUNCH_PATH, &want, 2)
                    .ok_or_else(|| Error::Other(format!("could not edit {}", cfg.display())))?;
                if first || !rec.launch.iter().any(|(p, _)| p == &cfg) {
                    rec.launch.push((cfg.clone(), before));
                }
                write_atomic(&cfg, &new)?;
                set = want;
            }
            rec.applied_at = now();
            save_record(ctx, FIX_LAUNCH, &rec)?;
            Ok(format!("Launch options set: {set}"))
        }
        FIX_HDR => {
            let plan = hdr::status(&ctx.probe.hdr, runner(ctx).as_ref(), true)
                .and_then(|s| s.plan)
                .ok_or_else(|| Error::Other("no HDR launch options for this system".into()))?;
            write_hdr(ctx, &plan)
        }
        FIX_OVERRIDES => {
            let prefix = g.proton_prefix.as_ref().ok_or_else(|| Error::Other("no prefix".into()))?;
            let reg_path = prefix.join("user.reg");
            let reg = std::fs::read_to_string(&reg_path).unwrap_or_default();
            let before = read_overrides(&reg);
            let mut rec = load_record(ctx, FIX_OVERRIDES).unwrap_or_default();
            let dlls = override_dlls(g);
            // Keep each DLL's setting from before it was first changed.
            for d in &dlls {
                if !rec.overrides.iter().any(|(k, _)| k == d) {
                    rec.overrides.push((d.to_string(), before.get(*d).cloned()));
                }
            }
            let set: Vec<(String, Option<String>)> = dlls.iter().map(|d| (d.to_string(), Some("native,builtin".to_string()))).collect();
            write_atomic(&reg_path, &write_overrides(&reg, &set))?;
            rec.applied_at = now();
            save_record(ctx, FIX_OVERRIDES, &rec)?;
            Ok(format!("{} now load native first.", dlls.join(", ")))
        }
        FIX_CASE => {
            let dups = case_duplicates(&g.path);
            let (moves, conflicts) = merge_dups(&g.path, &dups)?;
            let mut rec = load_record(ctx, FIX_CASE).unwrap_or_default();
            rec.moves.extend(moves.iter().cloned());
            rec.applied_at = now();
            save_record(ctx, FIX_CASE, &rec)?;
            let mut msg = format!("Moved {} file(s) into the folders the game reads.", moves.len());
            if !conflicts.is_empty() {
                msg.push_str(&format!(" Left {} that already exist there: {}", conflicts.len(), conflicts.join(", ")));
            }
            Ok(msg)
        }
        _ => Err(Error::Other(format!("unknown fix {id}"))),
    }
}

/// Microsoft's d3dcompiler_47 (not Wine's) is in the prefix.
pub fn compiler_ok(prefix: &Path) -> bool {
    let dll = prefix.join("drive_c/windows/system32/d3dcompiler_47.dll");
    dll.is_file() && !is_wine_builtin(&dll)
}

/// The winetricks verb a prefix fix runs.
fn tricks_verb(id: &str) -> &'static str {
    if id == FIX_D3DCOMPILER { "d3dcompiler_47" } else { "vcrun2022" }
}

fn tricks_backup_dir(ctx: &Ctx, id: &str) -> PathBuf {
    ctx.state_dir.join(id)
}

/// A DLL the fix `id` may write into the prefix (backed up first).
fn is_component_file(name: &str, id: &str) -> bool {
    let stems: &[&str] = if id == FIX_D3DCOMPILER { &["d3dcompiler_47"] } else { VC_FILE_STEMS };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".dll") && stems.iter().any(|s| lower.starts_with(s))
}

/// Install a Windows component into the prefix with protontricks or
/// winetricks: the Visual C++ runtime or the shader compiler.
fn apply_tricks(ctx: &Ctx, id: &str) -> Result<String> {
    let prefix = ctx.game.proton_prefix.clone().ok_or_else(|| Error::Other("no prefix".into()))?;
    let (tricks, wine) = vc_installer(ctx).map_err(Error::Other)?;
    let verb = tricks_verb(id);
    // Back up once: a second run keeps the copy from before the first.
    if load_record(ctx, id).is_none() {
        let backup = tricks_backup_dir(ctx, id);
        let _ = std::fs::remove_dir_all(&backup);
        let mut rec = Record { applied_at: now(), ..Default::default() };
        for dir in SYSTEM_DIRS {
            let mut names = Vec::new();
            if let Ok(rd) = std::fs::read_dir(prefix.join(dir)) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if is_component_file(&name, id) && e.file_type().is_ok_and(|t| t.is_file()) {
                        std::fs::create_dir_all(backup.join(dir))?;
                        std::fs::copy(e.path(), backup.join(dir).join(&name))?;
                        activity::record_path(Kind::Backup, "Saved a copy of", &e.path());
                        names.push(name);
                    }
                }
            }
            rec.vc_files.push((dir.to_string(), names));
        }
        std::fs::create_dir_all(&backup)?;
        for reg in ["user.reg", "system.reg"] {
            if prefix.join(reg).is_file() {
                std::fs::copy(prefix.join(reg), backup.join(reg))?;
                activity::record_path(Kind::Backup, "Saved a copy of", &prefix.join(reg));
            }
        }
        save_record(ctx, id, &rec)?;
    }

    let mut cmd = match &tricks {
        Tricks::Protontricks(p) => {
            let mut c = user_command(&p.to_string_lossy());
            c.args([STEAM_APP_ID, "-q", "--force", verb]);
            c
        }
        Tricks::ProtontricksFlatpak => {
            let mut c = user_command("flatpak");
            c.args(["run", PROTONTRICKS_FLATPAK, STEAM_APP_ID, "-q", "--force", verb]);
            c
        }
        Tricks::Winetricks(w) => {
            let mut c = user_command(&w.to_string_lossy());
            c.env("WINEPREFIX", &prefix).args(["-q", "--force", verb]);
            if let Some(wine) = &wine {
                c.env("WINE", wine);
                if let Some(server) = wine.parent().map(|d| d.join("wineserver")).filter(|p| p.is_file()) {
                    c.env("WINESERVER", server);
                }
            }
            c
        }
    };
    let log_name = format!("{id}.log");
    let log_path = ctx.state_dir.join(&log_name);
    std::fs::create_dir_all(&ctx.state_dir)?;
    let log = std::fs::File::create(&log_path)?;
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    activity::record_path(
        Kind::Setup,
        format!("Running {} {} (output in {log_name})", cmd.get_program().to_string_lossy(), args.join(" ")),
        &prefix,
    );
    let status = cmd.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log).status()?;
    activity::record(Kind::Setup, format!("{verb} installer finished: {status}"));
    if id == FIX_D3DCOMPILER && compiler_ok(&prefix) {
        return Ok("Microsoft's d3dcompiler_47 installed. ReShade uses it from the next game start.".into());
    }
    let vc = vc_state(&prefix);
    if id == FIX_VC && vc.ok() {
        let v = match vc {
            Vc::Native { version, .. } => format!(" {}.{}", version.0, version.1),
            _ => String::new(),
        };
        return Ok(format!("Visual C++ runtime{v} installed. Start the game to check CET and RED4ext."));
    }
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let tail: Vec<&str> = log.lines().rev().take(8).collect();
    Err(Error::Other(format!(
        "{verb} didn't install ({}). Undo restores the prefix. Last output:\n{}\nFull log: {}",
        if status.success() { "the DLLs are still not in place" } else { "the installer failed" },
        tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
        log_path.display()
    )))
}

/// Put back what a fix changed.
pub fn undo(ctx: &Ctx, id: &str) -> Result<String> {
    let all = checks(ctx);
    let check = find(&all, id)?;
    let undo = check.undo.as_ref().ok_or_else(|| Error::Other("nothing to undo".into()))?;
    if let Some(b) = &undo.blocked {
        return Err(Error::Other(b.clone()));
    }
    let rec = load_record(ctx, id).ok_or_else(|| Error::Other("nothing to undo".into()))?;
    let g = ctx.game;
    let msg = match id {
        FIX_VC | FIX_D3DCOMPILER => {
            let prefix = g.proton_prefix.clone().ok_or_else(|| Error::Other("no prefix".into()))?;
            let backup = tricks_backup_dir(ctx, id);
            for (dir, names) in &rec.vc_files {
                if let Ok(rd) = std::fs::read_dir(prefix.join(dir)) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if is_component_file(&name, id) && !names.contains(&name) {
                            std::fs::remove_file(e.path())?;
                            activity::record_path(Kind::Delete, "Removed", &e.path());
                        }
                    }
                }
                for name in names {
                    std::fs::copy(backup.join(dir).join(name), prefix.join(dir).join(name))?;
                    activity::record_path(Kind::Restore, "Put back", &prefix.join(dir).join(name));
                }
            }
            for reg in ["user.reg", "system.reg"] {
                if backup.join(reg).is_file() {
                    std::fs::copy(backup.join(reg), prefix.join(reg))?;
                    activity::record_path(Kind::Restore, "Put back", &prefix.join(reg));
                }
            }
            let _ = std::fs::remove_dir_all(&backup);
            if id == FIX_VC {
                "The prefix's Visual C++ runtime and registry are back to how they were.".to_string()
            } else {
                "The prefix's shader compiler and registry are back to how they were.".to_string()
            }
        }
        FIX_LAUNCH => {
            for (cfg, before) in &rec.launch {
                let Ok(src) = std::fs::read_to_string(cfg) else { continue };
                if let Some(new) = vdf::set_value(&src, LAUNCH_PATH, before.as_deref().unwrap_or(""), 2) {
                    write_atomic(cfg, &new)?;
                }
            }
            "The previous launch options are back.".into()
        }
        FIX_HDR => {
            for (cfg, _) in &rec.launch {
                let Ok(src) = std::fs::read_to_string(cfg) else { continue };
                let Some(cur) = launch_in(&src) else { continue };
                let want = hdr::strip(&cur);
                if want != cur
                    && let Some(new) = vdf::set_value(&src, LAUNCH_PATH, &want, 2)
                {
                    write_atomic(cfg, &new)?;
                }
            }
            "HDR settings removed from the launch options.".into()
        }
        FIX_FSR4 => {
            for (cfg, _) in &rec.launch {
                let Ok(src) = std::fs::read_to_string(cfg) else { continue };
                let Some(cur) = launch_in(&src) else { continue };
                let want = merge_env(&cur, crate::optiscaler::OVERRIDE_VAR, None);
                if want != cur
                    && let Some(new) = vdf::set_value(&src, LAUNCH_PATH, &want, 2)
                {
                    write_atomic(cfg, &new)?;
                }
            }
            "Shader overrides removed from the launch options.".into()
        }
        FIX_OVERRIDES => {
            let prefix = g.proton_prefix.as_ref().ok_or_else(|| Error::Other("no prefix".into()))?;
            let reg_path = prefix.join("user.reg");
            let reg = std::fs::read_to_string(&reg_path).unwrap_or_default();
            write_atomic(&reg_path, &write_overrides(&reg, &rec.overrides))?;
            "The previous DLL overrides are back.".into()
        }
        FIX_CASE => {
            let mut skipped = 0;
            for (from, to) in rec.moves.iter().rev() {
                let (src, dst) = (g.path.join(to), g.path.join(from));
                if !src.is_file() || dst.symlink_metadata().is_ok() {
                    skipped += 1;
                    continue;
                }
                if let Some(p) = dst.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::rename(&src, &dst)?;
                activity::record_path(Kind::Move, format!("Moved {} back to", src.display()), &dst);
            }
            if skipped > 0 {
                format!("Files moved back; {skipped} had changed since and were left where they are.")
            } else {
                "Files moved back to their old folders.".into()
            }
        }
        _ => return Err(Error::Other(format!("unknown fix {id}"))),
    };
    std::fs::remove_file(record_path(ctx, id))?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Framework, GAME_EXE};

    fn touch(p: &Path, content: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[cfg_attr(windows, allow(dead_code))]
    fn install(dir: &Path, store: Store, prefix: Option<PathBuf>, launch: Option<&str>) -> GameInstall {
        touch(&dir.join(GAME_EXE), b"x");
        GameInstall {
            path: dir.to_path_buf(),
            store,
            proton_prefix: prefix,
            build_id: None,
            exe_file_version: None,
            exe_product_version: None,
            frameworks: vec![Framework { id: "cet".into(), name: "CET".into(), installed: true }],
            launch_options: launch.map(String::from),
            warnings: vec![],
        }
    }

    #[test]
    #[cfg(not(windows))] // the checks are Linux-only
    fn reshade_needs_its_override_and_the_shader_compiler() {
        let gdir = tempfile::tempdir().unwrap();
        let pfx = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let g = install(gdir.path(), Store::Steam, Some(pfx.path().to_path_buf()), Some("WINEDLLOVERRIDES=\"winmm,version=n,b\" %command% --launcher-skip"));
        let ctx = Ctx { home: home.path(), game: &g, state_dir: state.path().to_path_buf(), probe: Probe::default() };
        assert!(checks(&ctx).iter().all(|c| c.id != FIX_D3DCOMPILER), "no ReShade, no compiler check");
        assert_eq!(checks(&ctx).iter().find(|c| c.id == FIX_LAUNCH).unwrap().state, State::Ok);

        touch(&gdir.path().join("bin/x64/dxgi.dll"), crate::reshade::tests::STAND_IN);
        assert_eq!(override_dlls(&g), ["winmm", "version", "dxgi", "d3dcompiler_47"]);
        let all = checks(&ctx);
        let launch = all.iter().find(|c| c.id == FIX_LAUNCH).unwrap();
        assert_eq!(launch.state, State::Problem);
        assert!(launch.detail.contains("winmm,version,dxgi,d3dcompiler_47=n,b") && launch.detail.contains("CET, RED4ext and ReShade"), "{}", launch.detail);
        let dc = all.iter().find(|c| c.id == FIX_D3DCOMPILER).unwrap();
        assert_eq!(dc.state, State::Problem);
        assert_eq!(dc.fix.as_ref().unwrap().label, "Install d3dcompiler_47");

        let mut builtin = vec![0u8; 0x80];
        builtin[..2].copy_from_slice(b"MZ");
        builtin[0x40..0x50].copy_from_slice(b"Wine builtin DLL");
        let dll = pfx.path().join("drive_c/windows/system32/d3dcompiler_47.dll");
        touch(&dll, &builtin);
        assert!(!compiler_ok(pfx.path()));
        touch(&dll, b"MZ microsoft's");
        assert!(compiler_ok(pfx.path()));
        assert_eq!(checks(&ctx).iter().find(|c| c.id == FIX_D3DCOMPILER).unwrap().state, State::Ok);
    }

    #[test]
    fn sets_and_removes_an_environment_variable() {
        const K: &str = "VKD3D_SHADER_OVERRIDE";
        assert_eq!(merge_env("", K, Some("Z:/a b")), "VKD3D_SHADER_OVERRIDE='Z:/a b' %command%");
        assert_eq!(merge_env("-skipStartScreen", K, Some("Z:/x")), "VKD3D_SHADER_OVERRIDE='Z:/x' %command% -skipStartScreen");
        let set = "VKD3D_SHADER_OVERRIDE='Z:/old dir' WINEDLLOVERRIDES=\"winmm=n,b\" %command% --launcher-skip";
        assert_eq!(
            merge_env(set, K, Some("Z:/new")),
            "VKD3D_SHADER_OVERRIDE='Z:/new' WINEDLLOVERRIDES=\"winmm=n,b\" %command% --launcher-skip"
        );
        assert_eq!(merge_env(set, K, None), "WINEDLLOVERRIDES=\"winmm=n,b\" %command% --launcher-skip");
        assert_eq!(merge_env("VKD3D_SHADER_OVERRIDE=Z:/x %command%", K, None), "");
        assert_eq!(find_env("XVKD3D_SHADER_OVERRIDE=1 %command%", K), None, "only whole names");
    }

    #[test]
    #[cfg(not(windows))]
    fn fsr4_overrides_check_follows_the_choice() {
        let gdir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let plain = "WINEDLLOVERRIDES=\"winmm,version=n,b\" %command% --launcher-skip";
        let g = install(gdir.path(), Store::Steam, None, Some(plain));
        let ctx = Ctx { home: home.path(), game: &g, state_dir: state.path().to_path_buf(), probe: Probe::default() };
        assert!(checks(&ctx).iter().all(|c| c.id != FIX_FSR4), "nothing picked: no check");

        std::fs::create_dir_all(root.path().join("prebuilt")).unwrap();
        crate::optiscaler::save_choice(state.path(), root.path(), Some(crate::optiscaler::Variant::Exact)).unwrap();
        let c = checks(&ctx).into_iter().find(|c| c.id == FIX_FSR4).unwrap();
        assert_eq!(c.state, State::Problem);
        let want = format!("VKD3D_SHADER_OVERRIDE='Z:{}' {plain}", root.path().join("prebuilt").display());
        assert_eq!(c.copy.as_deref(), Some(want.as_str()));
        assert_eq!(c.fix.unwrap().label, "Set launch option");

        let g2 = install(gdir.path(), Store::Steam, None, Some(&want));
        let ctx2 = Ctx { home: home.path(), game: &g2, state_dir: state.path().to_path_buf(), probe: Probe::default() };
        assert_eq!(checks(&ctx2).into_iter().find(|c| c.id == FIX_FSR4).unwrap().state, State::Ok);

        // Turned off while the launch options still carry it.
        crate::optiscaler::save_choice(state.path(), root.path(), None).unwrap();
        std::fs::write(state.path().join(format!("{FIX_FSR4}.json")), "{\"applied_at\":1}").unwrap();
        let c = checks(&ctx2).into_iter().find(|c| c.id == FIX_FSR4).unwrap();
        assert_eq!(c.state, State::Problem);
        assert_eq!(c.fix.unwrap().label, "Remove launch option");
    }

    #[test]
    fn optiscaler_needs_its_override() {
        let gdir = tempfile::tempdir().unwrap();
        let g = install(gdir.path(), Store::Steam, None, None);
        touch(&gdir.path().join("bin/x64/wininet.dll"), b"MZ");
        assert_eq!(override_dlls(&g), ["winmm", "version"], "no OptiScaler.ini: not OptiScaler");
        touch(&gdir.path().join("bin/x64/OptiScaler.ini"), b"[x]");
        assert_eq!(override_dlls(&g), ["winmm", "version", "wininet"]);
    }

    #[test]
    fn merges_launch_options() {
        const FW: &[&str] = &["winmm", "version"];
        let o = OVERRIDE_VALUE;
        assert_eq!(override_value(FW), o);
        assert_eq!(merge_launch_options("", FW, false), format!("WINEDLLOVERRIDES=\"{o}\" %command% --launcher-skip"));
        assert_eq!(merge_launch_options("", FW, true), format!("WINEDLLOVERRIDES=\"{o}\" %command% -modded --launcher-skip"));
        assert_eq!(
            merge_launch_options("-skipStartScreen", FW, false),
            format!("WINEDLLOVERRIDES=\"{o}\" %command% --launcher-skip -skipStartScreen")
        );
        assert_eq!(
            merge_launch_options("PROTON_LOG=1 %command% --launcher-skip", FW, true),
            format!("WINEDLLOVERRIDES=\"{o}\" PROTON_LOG=1 %command% -modded --launcher-skip")
        );
        // An existing override list keeps its other DLLs.
        assert_eq!(
            merge_launch_options("WINEDLLOVERRIDES=\"dxgi=n;winmm=b\" %command%", FW, false),
            format!("WINEDLLOVERRIDES=\"{o};dxgi=n\" %command% --launcher-skip")
        );
        assert_eq!(
            merge_launch_options("WINEDLLOVERRIDES=d3d11=n %command%", FW, false),
            format!("WINEDLLOVERRIDES=\"{o};d3d11=n\" %command% --launcher-skip")
        );
        let done = format!("WINEDLLOVERRIDES=\"{o}\" %command% -modded --launcher-skip");
        assert_eq!(merge_launch_options(&done, FW, true), done, "already right: unchanged");

        // ReShade adds its DLL and the shader compiler to the same list, and
        // takes dxgi out of an entry that set it differently.
        let all = &["winmm", "version", "dxgi", "d3dcompiler_47"];
        assert_eq!(
            merge_launch_options("WINEDLLOVERRIDES=\"dxgi=b;d3d11=n\" %command%", all, false),
            "WINEDLLOVERRIDES=\"winmm,version,dxgi,d3dcompiler_47=n,b;d3d11=n\" %command% --launcher-skip"
        );
        assert!(has_overrides("WINEDLLOVERRIDES=\"winmm,version=n,b;dxgi=n;d3dcompiler_47.dll=native\" %command%", all));
        assert!(!has_overrides("WINEDLLOVERRIDES=\"winmm,version=n,b;dxgi=b\" %command%", &["dxgi"]));
        assert!(!has_overrides("%command%", &["dxgi"]));
        assert!(has_overrides("%command%", &[]));
        assert_eq!(merge_launch_options("-skipStartScreen", &[], false), "%command% --launcher-skip -skipStartScreen");
        assert_eq!(merge_launch_options("%command% --LAUNCHER-SKIP", &[], false), "%command% --LAUNCHER-SKIP", "already skipped: unchanged");
        assert_eq!(remove_arg("A=1 %command% -modded --launcher-skip", MODDED), "A=1 %command% --launcher-skip");
        assert_eq!(remove_arg("%command% -modded", MODDED), "");
    }

    #[test]
    fn edits_wine_dll_overrides() {
        let reg = "WINE REGISTRY Version 2\n\n[Software\\\\Wine\\\\DllOverrides] 1700000000\n#time=1da\n\"*d3d11\"=\"native\"\n\"winmm\"=\"builtin\"\n\n[Software\\\\Wine\\\\Fonts] 1\n\"x\"=\"y\"\n";
        let o = read_overrides(reg);
        assert_eq!(o.get("d3d11").map(String::as_str), Some("native"));
        let set = vec![("winmm".to_string(), Some("native,builtin".to_string())), ("version".to_string(), Some("native,builtin".to_string()))];
        let out = write_overrides(reg, &set);
        let o2 = read_overrides(&out);
        assert_eq!(o2.get("winmm").map(String::as_str), Some("native,builtin"));
        assert_eq!(o2.get("version").map(String::as_str), Some("native,builtin"));
        assert!(out.contains("[Software\\\\Wine\\\\Fonts] 1\n\"x\"=\"y\""), "{out}");
        assert!(!read_overrides(&out).contains_key("x"));
        // Undo: back to what was there, and the key that wasn't is removed.
        let back = write_overrides(&out, &[("winmm".into(), Some("builtin".into())), ("version".into(), None)]);
        assert_eq!(read_overrides(&back), o);
        // A prefix without the section gets one.
        let fresh = write_overrides("WINE REGISTRY Version 2\n", &set);
        assert_eq!(read_overrides(&fresh).len(), 2);
    }

    #[test]
    fn detects_wine_builtin_runtime() {
        let pfx = tempfile::tempdir().unwrap();
        assert_eq!(vc_state(pfx.path()), Vc::Missing);
        let mut stub = vec![0u8; 0x80];
        stub[..2].copy_from_slice(b"MZ");
        stub[0x40..0x50].copy_from_slice(b"Wine builtin DLL");
        touch(&pfx.path().join("drive_c/windows/system32/msvcp140.dll"), &stub);
        assert_eq!(vc_state(pfx.path()), Vc::WineBuiltin);
        assert!(!vc_state(pfx.path()).ok());
    }

    #[test]
    #[cfg(not(windows))] // the checks are Linux-only
    fn steam_launch_options_fix_and_undo() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".local/share/Steam");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        let cfg = root.join("userdata/123/config/localconfig.vdf");
        let before = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1091500\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-skipStartScreen\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";
        touch(&cfg, before.as_bytes());
        let gdir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let g = install(gdir.path(), Store::Steam, None, Some("-skipStartScreen"));
        let mut ctx = Ctx { home: home.path(), game: &g, state_dir: state.path().to_path_buf(), probe: Probe { steam_running: true, ..Default::default() } };

        let c = checks(&ctx).into_iter().find(|c| c.id == FIX_LAUNCH).unwrap();
        assert_eq!(c.state, State::Problem);
        assert!(c.fix.as_ref().unwrap().blocked.as_deref().unwrap().contains("Quit Steam"));
        assert!(apply(&ctx, FIX_LAUNCH).is_err(), "never while Steam runs");
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), before);

        ctx.probe.steam_running = false;
        apply(&ctx, FIX_LAUNCH).unwrap();
        let now = launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(now, format!("WINEDLLOVERRIDES=\"{OVERRIDE_VALUE}\" %command% --launcher-skip -skipStartScreen"));

        let g2 = GameInstall { launch_options: Some(now), ..g.clone() };
        let ctx2 = Ctx { game: &g2, ..ctx };
        let c = checks(&ctx2).into_iter().find(|c| c.id == FIX_LAUNCH).unwrap();
        assert_eq!(c.state, State::Ok);
        assert!(c.undo.is_some());
        undo(&ctx2, FIX_LAUNCH).unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), before, "undo restores the file exactly");
    }

    #[test]
    #[cfg(not(windows))] // the checks are Linux-only
    fn hdr_launch_options_fix_and_undo() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".local/share/Steam");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        let cfg = root.join("userdata/123/config/localconfig.vdf");
        let mods = "WINEDLLOVERRIDES=\\\"winmm,version=n,b\\\" %command%";
        let vdf_with = |opts: &str| {
            format!("\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n\t\t\t\t\t\"1091500\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"{opts}\"\n\t\t\t\t\t}}\n\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n}}\n")
        };
        touch(&cfg, vdf_with(mods).as_bytes());
        let before = launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        // The prefix was last run by GE-Proton.
        let compat = tempfile::tempdir().unwrap();
        let pfx = compat.path().join("pfx");
        std::fs::create_dir_all(&pfx).unwrap();
        touch(&compat.path().join("config_info"), b"GE-Proton10-25\n/home/u/.steam/root/compatibilitytools.d/GE-Proton10-25/files/share/fonts/\n");
        let gdir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let g = install(gdir.path(), Store::Steam, Some(pfx), Some(&before));
        let hdr = hdr::Probe {
            displays: vec![hdr::Display { connector: "DP-1".into(), name: None, pq: true, mode: Some((3840, 2160)) }],
            wayland: true,
            desktop: "KDE".into(),
            desktop_version: None,
            desktop_hdr: Some(true),
            gpu: Some(hdr::Gpu::Mesa("25.2.3".into())),
            hdr_layer: false,
            gamescope: false,
        };
        let ctx = Ctx { home: home.path(), game: &g, state_dir: state.path().to_path_buf(), probe: Probe { hdr, ..Default::default() } };

        let c = checks(&ctx).into_iter().find(|c| c.id == FIX_HDR).unwrap();
        assert_eq!(c.state, State::Problem);
        assert!(c.detail.contains("GE-Proton10-25 (Wine Wayland driver)") && c.detail.contains("lack PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1"), "{}", c.detail);
        apply(&ctx, FIX_HDR).unwrap();
        let now = launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(now, format!("PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 {before}"));

        let g2 = GameInstall { launch_options: Some(now), ..g.clone() };
        let ctx2 = Ctx { game: &g2, ..ctx };
        let c = checks(&ctx2).into_iter().find(|c| c.id == FIX_HDR).unwrap();
        assert_eq!(c.state, State::Ok, "{}", c.detail);
        assert!(c.fix.is_none() && c.undo.is_some());
        assert!(c.alt.is_none(), "no gamescope installed, no second method");
        undo(&ctx2, FIX_HDR).unwrap();
        assert_eq!(launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap(), before);

        // With gamescope installed too, either method can be picked.
        let mut ctx3 = Ctx { game: &g, ..ctx2 };
        ctx3.probe.hdr.gamescope = true;
        let c = checks(&ctx3).into_iter().find(|c| c.id == FIX_HDR).unwrap();
        assert_eq!(c.alt.as_ref().unwrap().label, "Set HDR with gamescope");
        assert_eq!(c.alt_id.as_deref(), Some(FIX_HDR_GAMESCOPE));
        apply(&ctx3, FIX_HDR_GAMESCOPE).unwrap();
        let gs = launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert!(gs.starts_with("DXVK_HDR=1 ") && gs.contains("--hdr-enabled --force-grab-cursor -- %command%") && !gs.contains("WAYLAND"), "{gs}");
        let g4 = GameInstall { launch_options: Some(gs), ..g.clone() };
        let ctx4 = Ctx { game: &g4, ..ctx3 };
        let c = checks(&ctx4).into_iter().find(|c| c.id == FIX_HDR).unwrap();
        assert_eq!(c.state, State::Ok);
        assert_eq!(c.alt.as_ref().unwrap().label, "Use Wayland instead");
        apply(&ctx4, FIX_HDR_WAYLAND).unwrap();
        assert_eq!(launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap(), format!("PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 {before}"));
        undo(&ctx4, FIX_HDR).unwrap();
        assert_eq!(launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap(), before);
    }

    #[test]
    fn redmod_switch_sets_modded_and_skips_the_launcher() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".local/share/Steam");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        let cfg = root.join("userdata/123/config/localconfig.vdf");
        let vdf_with = |opts: &str| {
            format!("\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n\t\t\t\t\t\"1091500\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"{opts}\"\n\t\t\t\t\t}}\n\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n}}\n")
        };
        touch(&cfg, vdf_with("DXVK_HDR=1 %command%").as_bytes());
        let gdir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let g = install(gdir.path(), Store::Steam, None, Some("DXVK_HDR=1 %command%"));
        let mut ctx = Ctx { home: home.path(), game: &g, state_dir: state.path().to_path_buf(), probe: Probe::default() };
        assert!(redmod(&ctx).is_none(), "no REDmod tool, no switch");

        touch(&gdir.path().join("tools/redmod/bin/redMod.exe"), b"x");
        touch(&gdir.path().join("mods/a/info.json"), b"{}");
        let r = redmod(&ctx).unwrap();
        assert!(!r.enabled && r.steam && r.mods == 1 && r.blocked.is_none());

        ctx.probe.steam_running = true;
        assert!(redmod(&ctx).unwrap().blocked.is_some());
        assert!(set_redmod(&ctx, true).is_err(), "never while Steam runs");
        ctx.probe.steam_running = false;

        set_redmod(&ctx, true).unwrap();
        let on = launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(on, "DXVK_HDR=1 %command% -modded --launcher-skip");
        assert!(redmod_wanted(&ctx));

        set_redmod(&ctx, false).unwrap();
        let off = launch_in(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(off, "DXVK_HDR=1 %command% --launcher-skip");
        assert!(!redmod_wanted(&ctx), "off is kept");
        if cfg!(not(windows)) {
            // Set launch options no longer asks for -modded once REDmod is off.
            let g2 = GameInstall { launch_options: Some(off), frameworks: vec![], ..g.clone() };
            let ctx2 = Ctx { game: &g2, ..ctx };
            let c = checks(&ctx2).into_iter().find(|c| c.id == FIX_LAUNCH).unwrap();
            assert_eq!(c.state, State::Ok, "{}", c.detail);
        }

        let gog = install(gdir.path(), Store::Gog, None, None);
        let ctx3 = Ctx { home: home.path(), game: &gog, state_dir: state.path().to_path_buf(), probe: Probe::default() };
        let r = redmod(&ctx3).unwrap();
        assert!(!r.steam && r.copy.as_deref() == Some("-modded --launcher-skip"));
        assert!(set_redmod(&ctx3, true).is_err());
    }

    #[test]
    #[cfg(not(windows))] // the checks are Linux-only
    fn heroic_prefix_gets_registry_overrides() {
        let home = tempfile::tempdir().unwrap();
        let gdir = tempfile::tempdir().unwrap();
        let pfx = tempfile::tempdir().unwrap();
        touch(&pfx.path().join("user.reg"), b"WINE REGISTRY Version 2\n");
        let state = tempfile::tempdir().unwrap();
        let g = install(gdir.path(), Store::Gog, Some(pfx.path().to_path_buf()), None);
        let ctx = Ctx { home: home.path(), game: &g, state_dir: state.path().to_path_buf(), probe: Probe::default() };
        let all = checks(&ctx);
        assert!(all.iter().all(|c| c.id != FIX_LAUNCH), "no Steam options for GOG");
        let vc = all.iter().find(|c| c.id == FIX_VC).unwrap();
        assert!(vc.fix.as_ref().unwrap().blocked.as_deref().unwrap().contains("winetricks"));
        apply(&ctx, FIX_OVERRIDES).unwrap();
        assert_eq!(checks(&ctx).into_iter().find(|c| c.id == FIX_OVERRIDES).unwrap().state, State::Ok);
        undo(&ctx, FIX_OVERRIDES).unwrap();
        assert!(read_overrides(&std::fs::read_to_string(pfx.path().join("user.reg")).unwrap()).is_empty());
    }

    #[test]
    #[cfg(not(windows))] // the checks are Linux-only
    fn merges_case_duplicate_folders_and_moves_back() {
        let home = tempfile::tempdir().unwrap();
        let gdir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let game = gdir.path();
        touch(&game.join("archive/pc/mod/a.archive"), b"a");
        touch(&game.join("Archive/PC/Mod/b.archive"), b"b");
        touch(&game.join("Archive/PC/Mod/a.archive"), b"other a");
        touch(&game.join("r6/scripts/x.reds"), b"x");
        let g = install(game, Store::Manual, None, None);
        let dups = case_duplicates(game);
        assert_eq!(dups, vec![CaseDup { keep: "archive".into(), others: vec!["Archive".into()] }]);

        let ctx = Ctx { home: home.path(), game: &g, state_dir: state.path().to_path_buf(), probe: Probe::default() };
        let msg = apply(&ctx, FIX_CASE).unwrap();
        assert!(msg.contains("Moved 1") && msg.contains("Archive/PC/Mod/a.archive"), "{msg}");
        assert_eq!(std::fs::read(game.join("archive/pc/mod/b.archive")).unwrap(), b"b");
        assert_eq!(std::fs::read(game.join("archive/pc/mod/a.archive")).unwrap(), b"a", "conflict left alone");

        undo(&ctx, FIX_CASE).unwrap();
        assert_eq!(std::fs::read(game.join("Archive/PC/Mod/b.archive")).unwrap(), b"b");
        assert!(!game.join("archive/pc/mod/b.archive").exists());
    }

    #[test]
    #[cfg(unix)]
    fn vc_fix_backs_up_runs_winetricks_and_undo_restores() {
        let home = tempfile::tempdir().unwrap();
        let gdir = tempfile::tempdir().unwrap();
        let pfx = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let sys32 = pfx.path().join("drive_c/windows/system32");
        touch(&sys32.join("msvcp140.dll"), b"old");
        touch(&pfx.path().join("user.reg"), b"WINE REGISTRY Version 2\n");
        // A stand-in winetricks that writes a runtime and touches the registry.
        let bin = tempfile::tempdir().unwrap();
        let wt = bin.path().join("winetricks");
        touch(&wt, b"#!/bin/sh\n[ -n \"$WINE\" ] || exit 3\necho new > \"$WINEPREFIX/drive_c/windows/system32/msvcp140.dll\"\necho new > \"$WINEPREFIX/drive_c/windows/system32/vcruntime140_1.dll\"\necho changed >> \"$WINEPREFIX/user.reg\"\n");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wt, std::fs::Permissions::from_mode(0o755)).unwrap();
        let wine = bin.path().join("wine");
        touch(&wine, b"");
        let g = install(gdir.path(), Store::Gog, Some(pfx.path().to_path_buf()), None);
        let mut ctx = Ctx {
            home: home.path(),
            game: &g,
            state_dir: state.path().to_path_buf(),
            probe: Probe { winetricks: Some(wt), cabextract: true, ..Default::default() },
        };
        // Without Heroic's settings there's no Wine build to use.
        let c = checks(&ctx).into_iter().find(|c| c.id == FIX_VC).unwrap();
        assert!(c.fix.unwrap().blocked.is_some());

        let cfg = home.path().join(".config/heroic");
        touch(
            &cfg.join("gog_store/installed.json"),
            serde_json::json!({"installed": [{"appName": "1423049311", "install_path": gdir.path()}]}).to_string().as_bytes(),
        );
        touch(
            &cfg.join("GamesConfig/1423049311.json"),
            serde_json::json!({"1423049311": {"winePrefix": pfx.path(), "wineVersion": {"bin": wine, "type": "wine"}}}).to_string().as_bytes(),
        );
        ctx.probe.game_running = true;
        assert!(apply(&ctx, FIX_VC).unwrap_err().to_string().contains("Quit the game"));
        ctx.probe.game_running = false;

        // The stand-in writes no real DLL, so the check after still fails,
        // and says Undo is there.
        let err = apply(&ctx, FIX_VC).unwrap_err().to_string();
        assert!(err.contains("Undo"), "{err}");
        assert_eq!(std::fs::read(sys32.join("msvcp140.dll")).unwrap(), b"new\n");

        undo(&ctx, FIX_VC).unwrap();
        assert_eq!(std::fs::read(sys32.join("msvcp140.dll")).unwrap(), b"old");
        assert!(!sys32.join("vcruntime140_1.dll").exists(), "files the installer added are removed");
        assert_eq!(std::fs::read_to_string(pfx.path().join("user.reg")).unwrap(), "WINE REGISTRY Version 2\n");
        assert!(checks(&ctx).into_iter().find(|c| c.id == FIX_VC).unwrap().undo.is_none());
    }
}
