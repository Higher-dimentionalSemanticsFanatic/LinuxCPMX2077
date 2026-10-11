//! HDR for the game under Proton on Linux.
//!
//! Cyberpunk 2077 offers its HDR modes only when DXGI reports an HDR colour
//! space. Under Proton that takes, all at once:
//!
//! - a display whose EDID lists the PQ (SMPTE ST 2084) transfer function;
//! - a Wayland session with HDR on in the desktop's display settings (X11
//!   has no HDR output);
//! - a Vulkan driver with HDR swapchains on Wayland: Mesa 25.1+ or NVIDIA
//!   595+, older ones only through `vk_hdr_layer` and `ENABLE_HDR_WSI=1`;
//! - either a Proton build with Wine's Wayland driver (GE-Proton 10+ and
//!   proton-cachyos: `PROTON_ENABLE_WAYLAND=1`) plus `DXVK_HDR=1`, or the
//!   game wrapped in `gamescope --hdr-enabled` (Valve's Proton has no
//!   Wayland driver).
//!
//! `DXVK_HDR=1` also covers vkd3d-proton (the game is D3D12), which takes
//! its DXGI from DXVK. GE-Proton's `PROTON_ENABLE_HDR=1` only sets it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::desktop::user_command;

/// Launch option keys this module adds, for removing them again.
pub const ENV_KEYS: &[&str] = &["PROTON_ENABLE_WAYLAND", "PROTON_ENABLE_HDR", "DXVK_HDR", "ENABLE_HDR_WSI"];
const MESA_MIN: (u32, u32) = (25, 1);
const NVIDIA_MIN: u32 = 595;
const GNOME_MIN: u32 = 48;

/// One connected display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Display {
    /// DRM connector, e.g. `DP-1`.
    pub connector: String,
    /// Monitor name from the EDID.
    pub name: Option<String>,
    /// The EDID's HDR static metadata block lists PQ (HDR10).
    pub pq: bool,
    /// Preferred mode, for gamescope's output size.
    pub mode: Option<(u32, u32)>,
}

impl Display {
    fn label(&self) -> String {
        match &self.name {
            Some(n) => format!("{} {n}", self.connector),
            None => self.connector.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Gpu {
    Mesa(String),
    Nvidia(String),
}

impl Gpu {
    /// HDR swapchains on Wayland without `vk_hdr_layer`.
    fn native_hdr(&self) -> bool {
        match self {
            Gpu::Mesa(v) => version(v).is_some_and(|(a, b, _)| (a, b) >= MESA_MIN),
            Gpu::Nvidia(v) => version(v).is_some_and(|(a, ..)| a >= NVIDIA_MIN),
        }
    }

    fn label(&self) -> String {
        match self {
            Gpu::Mesa(v) => format!("Mesa {v}"),
            Gpu::Nvidia(v) => format!("NVIDIA {v}"),
        }
    }
}

/// What this machine offers for HDR. Tests build one by hand.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub displays: Vec<Display>,
    /// `XDG_SESSION_TYPE` / `WAYLAND_DISPLAY` say Wayland.
    pub wayland: bool,
    /// First entry of `XDG_CURRENT_DESKTOP`, e.g. `KDE`, `GNOME`, `Hyprland`.
    pub desktop: String,
    pub desktop_version: Option<String>,
    /// HDR on for some display, where the desktop reports it (KDE).
    pub desktop_hdr: Option<bool>,
    pub gpu: Option<Gpu>,
    /// An implicit Vulkan layer providing `ENABLE_HDR_WSI` is installed.
    pub hdr_layer: bool,
    pub gamescope: bool,
}

impl Probe {
    /// Reads `/sys/class/drm`; runs desktop and Vulkan tools only when an HDR
    /// display is connected. Kept for a short while, since every setup check
    /// asks. Empty on Windows.
    pub fn system() -> Self {
        static CACHE: Mutex<Option<(Instant, Probe)>> = Mutex::new(None);
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, p)) = cache.as_ref()
            && at.elapsed() < Duration::from_secs(15)
        {
            return p.clone();
        }
        let p = Self::read();
        *cache = Some((Instant::now(), p.clone()));
        p
    }

    fn read() -> Self {
        if cfg!(windows) {
            return Probe::default();
        }
        let displays = displays(Path::new("/sys/class/drm"));
        if !displays.iter().any(|d| d.pq) {
            return Probe { displays, ..Default::default() };
        }
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        let wayland = env("XDG_SESSION_TYPE").eq_ignore_ascii_case("wayland") || !env("WAYLAND_DISPLAY").is_empty();
        let desktop = env("XDG_CURRENT_DESKTOP").split(':').next().unwrap_or_default().to_string();
        let (desktop_version, desktop_hdr) = match desktop.to_ascii_lowercase().as_str() {
            "kde" => (
                run("plasmashell", &["--version"]).and_then(|o| last_word(&o)),
                run("kscreen-doctor", &["-o"]).and_then(|o| kscreen_hdr(&o)),
            ),
            "gnome" => (run("gnome-shell", &["--version"]).and_then(|o| last_word(&o)), None),
            _ => (None, None),
        };
        let gpu = std::fs::read_to_string("/proc/driver/nvidia/version")
            .ok()
            .and_then(|s| nvidia_version(&s))
            .map(Gpu::Nvidia)
            .or_else(|| run("vulkaninfo", &["--summary"]).and_then(|o| vulkan_driver(&o)));
        Probe {
            displays,
            wayland,
            desktop,
            desktop_version,
            desktop_hdr,
            gpu,
            hdr_layer: hdr_layer_installed(),
            gamescope: crate::linux_setup::on_path("gamescope"),
        }
    }

    fn hdr_display(&self) -> Option<&Display> {
        self.displays.iter().find(|d| d.pq)
    }
}

/// Run a tool for its output, giving up after a few seconds.
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut child = user_command(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

fn last_word(s: &str) -> Option<String> {
    s.split_whitespace().last().map(String::from)
}

/// `(major, minor, patch)` from the start of `25.1.4-arch1.1` or `595.58.03`.
fn version(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(0));
    Some((it.next()?, it.next().unwrap_or(0), it.next().unwrap_or(0)))
}

// ---- Displays --------------------------------------------------------------------

/// Connected displays under a DRM class folder (`/sys/class/drm`).
pub fn displays(drm: &Path) -> Vec<Display> {
    let Ok(rd) = std::fs::read_dir(drm) else { return vec![] };
    let mut out: Vec<Display> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            // card0-DP-1: a connector, not the card itself.
            let (_, connector) = name.split_once('-')?;
            let dir = e.path();
            if std::fs::read_to_string(dir.join("status")).ok()?.trim() != "connected" {
                return None;
            }
            let edid = std::fs::read(dir.join("edid")).unwrap_or_default();
            let mode = std::fs::read_to_string(dir.join("modes")).ok().and_then(|m| {
                let (w, h) = m.lines().next()?.split_once('x')?;
                Some((w.trim().parse().ok()?, h.trim().trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()?))
            });
            Some(Display { connector: connector.to_string(), name: edid_name(&edid), pq: edid_pq(&edid), mode })
        })
        .collect();
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}

/// The monitor name descriptor (tag 0xFC) of an EDID base block.
pub fn edid_name(edid: &[u8]) -> Option<String> {
    (0..4).find_map(|i| {
        let d = edid.get(54 + i * 18..72 + i * 18)?;
        (d[..3] == [0, 0, 0] && d[3] == 0xFC).then(|| String::from_utf8_lossy(&d[5..]).split('\n').next().unwrap_or("").trim().to_string())
    })
    .filter(|n| !n.is_empty())
}

/// Whether any CTA-861 extension block's HDR static metadata data block
/// (extended tag 6) lists the SMPTE ST 2084 (PQ) transfer function.
pub fn edid_pq(edid: &[u8]) -> bool {
    edid.chunks(128).skip(1).any(|ext| {
        if ext.len() < 4 || ext[0] != 0x02 {
            return false;
        }
        let end = (ext[2] as usize).min(ext.len());
        let mut i = 4;
        while i < end {
            let (tag, len) = (ext[i] >> 5, (ext[i] & 0x1F) as usize);
            if tag == 7 && len >= 2 && ext.get(i + 1) == Some(&6) && ext.get(i + 2).is_some_and(|eotf| eotf & 0b100 != 0) {
                return true;
            }
            i += 1 + len;
        }
        false
    })
}

// ---- Desktop and driver ----------------------------------------------------------

/// `Some(true)` when any output in `kscreen-doctor -o` reports `HDR: enabled`.
pub fn kscreen_hdr(out: &str) -> Option<bool> {
    let plain = strip_ansi(out);
    let states: Vec<&str> = plain.match_indices("HDR:").filter_map(|(i, _)| plain[i + 4..].split_whitespace().next()).collect();
    if states.is_empty() {
        return None;
    }
    Some(states.iter().any(|s| s.eq_ignore_ascii_case("enabled")))
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            for c in it.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The version in `/proc/driver/nvidia/version`.
pub fn nvidia_version(s: &str) -> Option<String> {
    let line = s.lines().next()?;
    line.split_whitespace().find(|w| w.contains('.') && w.chars().next().is_some_and(|c| c.is_ascii_digit())).map(String::from)
}

/// The first GPU's driver from `vulkaninfo --summary`, skipping llvmpipe.
pub fn vulkan_driver(out: &str) -> Option<Gpu> {
    out.lines().filter_map(|l| l.trim().strip_prefix("driverInfo")).map(|v| v.trim_start_matches([' ', '=']).trim()).find_map(|v| {
        if let Some(m) = v.strip_prefix("Mesa ") {
            Some(Gpu::Mesa(m.split_whitespace().next()?.to_string()))
        } else if v.chars().next().is_some_and(|c| c.is_ascii_digit()) && !v.contains("LLVM") {
            Some(Gpu::Nvidia(v.split_whitespace().next()?.to_string()))
        } else {
            None
        }
    })
}

fn hdr_layer_installed() -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    ["/usr/share/vulkan/implicit_layer.d", "/etc/vulkan/implicit_layer.d"]
        .iter()
        .map(PathBuf::from)
        .chain([home.join(".local/share/vulkan/implicit_layer.d")])
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .any(|e| std::fs::read_to_string(e.path()).is_ok_and(|s| s.contains("ENABLE_HDR_WSI")))
}

// ---- Proton build ----------------------------------------------------------------

/// The Wine/Proton build that runs the game, by the folder it lives in.
#[derive(Debug, Clone, PartialEq)]
pub struct Runner {
    pub name: String,
    /// Has Wine's Wayland driver, switched on with `PROTON_ENABLE_WAYLAND=1`.
    pub wayland: bool,
}

/// Classify a Proton/Wine path such as `…/compatibilitytools.d/GE-Proton10-25/files`.
pub fn runner(path: &Path) -> Runner {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let pick = parts
        .iter()
        .rev()
        .find(|p| {
            let l = p.to_ascii_lowercase();
            l.contains("proton") || l.contains("wine-")
        })
        .cloned()
        .unwrap_or_else(|| path.display().to_string());
    let l = pick.to_ascii_lowercase();
    let ge = l.find("ge-proton").and_then(|i| version(&l[i + 9..])).is_some_and(|(major, ..)| major >= 10) && !l.starts_with("wine-");
    Runner { wayland: ge || l.contains("cachyos"), name: pick }
}

// ---- Plan ------------------------------------------------------------------------

/// Keeps the mouse captured inside gamescope. Without it the camera stops
/// turning once the cursor reaches the edge of the gamescope window.
pub const GRAB_CURSOR: &str = "--force-grab-cursor";

/// What the launch options need for HDR.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub env: Vec<(&'static str, &'static str)>,
    /// `gamescope … --` placed before `%command%`, when the Proton build has
    /// no Wayland driver.
    pub gamescope: Option<String>,
}

impl Plan {
    /// The variables and wrapper, as typed into launch options.
    pub fn line(&self) -> String {
        let mut s: Vec<String> = self.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if let Some(g) = &self.gamescope {
            s.push(g.clone());
        }
        s.join(" ")
    }
}

/// The result of checking one game for HDR.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    /// What was found, one item each.
    pub found: Vec<String>,
    /// Requirements outside the launch options that are not met.
    pub missing: Vec<String>,
    pub plan: Option<Plan>,
    /// The gamescope wrapper as a second choice, when the Proton build also
    /// has the Wine Wayland driver.
    pub alt: Option<Plan>,
    /// The desktop reports HDR off on every display: the game runs in SDR
    /// by the user's choice, so nothing is lost.
    pub sdr: bool,
}

/// `None` when no connected display reports HDR in its EDID.
pub fn status(p: &Probe, runner: Option<&Runner>, steam: bool) -> Option<Status> {
    let display = p.hdr_display()?;
    let mut found = vec![format!("Display {} (EDID: HDR10/PQ)", display.label())];
    let mut missing = Vec::new();

    let desktop = if p.desktop.is_empty() { "desktop not reported".to_string() } else { p.desktop.clone() };
    let desktop = match &p.desktop_version {
        Some(v) => format!("{desktop} {v}"),
        None => desktop,
    };
    if !p.wayland {
        found.push(format!("X11 session, {desktop}"));
        missing.push("X11 has no HDR output: log in to a Wayland session (KDE Plasma 6, GNOME 48 or newer)".into());
    } else {
        found.push(format!(
            "Wayland session, {desktop}, desktop HDR {}",
            match p.desktop_hdr {
                Some(true) => "enabled",
                Some(false) => "disabled",
                None => "not checked",
            }
        ));
        if p.desktop_hdr == Some(false) {
            missing.push("Desktop HDR disabled: System Settings › Display & Monitor › Enable HDR".into());
        }
        if p.desktop.eq_ignore_ascii_case("gnome") && p.desktop_version.as_deref().and_then(version).is_some_and(|(m, ..)| m < GNOME_MIN) {
            missing.push(format!("HDR output requires GNOME {GNOME_MIN} or newer"));
        }
    }

    let layer = match &p.gpu {
        Some(g) if g.native_hdr() => {
            found.push(format!("{} (Vulkan HDR on Wayland)", g.label()));
            false
        }
        Some(g) => {
            let need = match g {
                Gpu::Mesa(_) => format!("Mesa {}.{}", MESA_MIN.0, MESA_MIN.1),
                Gpu::Nvidia(_) => format!("NVIDIA {NVIDIA_MIN}"),
            };
            if p.hdr_layer {
                found.push(format!("{} with vk_hdr_layer", g.label()));
            } else {
                found.push(g.label());
                missing.push(format!("{}: Vulkan HDR on Wayland requires {need} or newer, or vk_hdr_layer", g.label()));
            }
            p.hdr_layer
        }
        None => {
            found.push(format!("GPU driver not checked (needs Mesa {}.{}+ or NVIDIA {NVIDIA_MIN}+)", MESA_MIN.0, MESA_MIN.1));
            false
        }
    };

    let (w, h) = display.mode.unwrap_or((3840, 2160));
    let gamescope = Plan { env: vec![("DXVK_HDR", "1")], gamescope: Some(format!("gamescope -W {w} -H {h} -f --hdr-enabled {GRAB_CURSOR} --")) };
    let mut alt = None;
    let plan = match runner {
        None => {
            missing.push("Proton build not detected: start the game once, then rescan".into());
            None
        }
        Some(r) if r.wayland => {
            found.push(format!("{} (Wine Wayland driver)", r.name));
            let mut env = vec![("PROTON_ENABLE_WAYLAND", "1"), ("DXVK_HDR", "1")];
            if layer {
                env.push(("ENABLE_HDR_WSI", "1"));
            }
            if p.gamescope && steam {
                alt = Some(gamescope.clone());
            }
            Some(Plan { env, gamescope: None })
        }
        Some(r) if p.gamescope && steam => {
            found.push(format!("{} (no Wine Wayland driver), gamescope installed", r.name));
            Some(gamescope)
        }
        Some(r) => {
            found.push(format!("{} (no Wine Wayland driver)", r.name));
            missing.push(if steam {
                format!("{}: no Wine Wayland driver. Select GE-Proton 10 or newer (Steam › Properties › Compatibility), or install gamescope", r.name)
            } else {
                format!("{}: no Wine Wayland driver. Select GE-Proton 10 or newer as the Wine version in Heroic", r.name)
            });
            None
        }
    };
    Some(Status { found, missing, plan, alt, sdr: p.wayland && p.desktop_hdr == Some(false) })
}

// ---- Launch options ----------------------------------------------------------------

fn key_of(token: &str) -> Option<&str> {
    let (k, _) = token.split_once('=')?;
    (!k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then_some(k)
}

/// The launch options already carry every variable and the wrapper of `plan`.
pub fn has_plan(opts: &str, plan: &Plan) -> bool {
    let toks: Vec<&str> = opts.split_whitespace().collect();
    plan.env.iter().all(|(k, v)| toks.contains(&format!("{k}={v}").as_str()))
        && plan.gamescope.as_ref().is_none_or(|g| {
            toks.contains(&"gamescope") && toks.contains(&"--hdr-enabled") && (!g.contains(GRAB_CURSOR) || toks.contains(&GRAB_CURSOR))
        })
}

/// The launch options with `plan` added, keeping everything else. Bare
/// options are game arguments and go after `%command%`.
pub fn merge(opts: &str, plan: &Plan) -> String {
    let opts = opts.trim();
    let mut s = if opts.contains("%command%") { opts.to_string() } else { format!("%command% {opts}").trim().to_string() };
    if let Some(g) = &plan.gamescope {
        let toks: Vec<&str> = s.split_whitespace().collect();
        if !toks.contains(&"gamescope") {
            s = s.replacen("%command%", &format!("{g} %command%"), 1);
        } else if g.contains(GRAB_CURSOR) && toks.contains(&"--hdr-enabled") && !toks.contains(&GRAB_CURSOR) {
            // Our wrapper from before the cursor grab was added.
            s = s.replacen("--hdr-enabled", &format!("--hdr-enabled {GRAB_CURSOR}"), 1);
        }
    }
    let have: Vec<String> = s.split_whitespace().map(String::from).collect();
    let add: Vec<String> = plan
        .env
        .iter()
        .filter(|(k, _)| !have.iter().any(|t| key_of(t) == Some(k)))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    // Wrong values for our keys (DXVK_HDR=0) are replaced in place.
    let s = s
        .split(' ')
        .map(|t| match plan.env.iter().find(|(k, _)| key_of(t) == Some(k)) {
            Some((k, v)) => format!("{k}={v}"),
            None => t.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    if add.is_empty() { s } else { format!("{} {s}", add.join(" ")) }
}

/// The launch options without the HDR variables and an HDR gamescope wrapper.
pub fn strip(opts: &str) -> String {
    let mut toks: Vec<&str> = opts.split_whitespace().filter(|t| key_of(t).is_none_or(|k| !ENV_KEYS.contains(&k))).collect();
    if let Some(g) = toks.iter().position(|t| *t == "gamescope")
        && let Some(end) = toks[g..].iter().position(|t| *t == "--").map(|e| g + e)
        && toks[g..end].contains(&"--hdr-enabled")
    {
        toks.drain(g..=end);
    }
    let s = toks.join(" ");
    if s == "%command%" { String::new() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A base EDID block named `name` plus one CTA extension with an HDR
    /// static metadata block listing `eotf`.
    fn edid(name: &str, eotf: u8) -> Vec<u8> {
        let mut e = vec![0u8; 256];
        e[..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
        e[126] = 1;
        let d = &mut e[72..90];
        d[3] = 0xFC;
        let n = format!("{name}\n");
        d[5..5 + n.len()].copy_from_slice(n.as_bytes());
        let x = &mut e[128..];
        x[0] = 0x02;
        x[1] = 3;
        // A video data block (tag 2, 2 bytes), then the extended block.
        x[4..7].copy_from_slice(&[0x42, 16, 4]);
        x[7..11].copy_from_slice(&[0xE3, 6, eotf, 1]);
        x[2] = 11;
        e
    }

    #[test]
    fn reads_hdr_and_name_from_edid() {
        assert!(edid_pq(&edid("LG ULTRAGEAR", 0b0101)));
        assert!(!edid_pq(&edid("SDR", 0b0001)), "SDR and HLG only");
        assert!(!edid_pq(&edid("x", 0)[..128]), "no extension");
        assert_eq!(edid_name(&edid("LG ULTRAGEAR", 5)).as_deref(), Some("LG ULTRAGEAR"));
    }

    #[test]
    fn lists_connected_displays_from_sysfs() {
        let drm = tempfile::tempdir().unwrap();
        let add = |name: &str, status: &str, e: &[u8], modes: &str| {
            let d = drm.path().join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("status"), status).unwrap();
            std::fs::write(d.join("edid"), e).unwrap();
            std::fs::write(d.join("modes"), modes).unwrap();
        };
        add("card1-DP-1", "connected\n", &edid("OLED", 4), "3440x1440\n2560x1440\n");
        add("card1-HDMI-A-1", "disconnected\n", &[], "");
        add("card1-DP-2", "connected\n", &edid("Office", 1), "1920x1080\n");
        std::fs::create_dir_all(drm.path().join("card1")).unwrap();
        let ds = displays(drm.path());
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0], Display { connector: "DP-1".into(), name: Some("OLED".into()), pq: true, mode: Some((3440, 1440)) });
        assert!(!ds[1].pq);
    }

    #[test]
    fn parses_tool_output() {
        let kde = "Output: 1 DP-1 enabled connected priority 1\n\tHDR: \u{1b}[01;32menabled\u{1b}[0m\n\tWide Color Gamut: enabled\nOutput: 2 DP-2\n\tHDR: incapable\n";
        assert_eq!(kscreen_hdr(kde), Some(true));
        assert_eq!(kscreen_hdr("Output: 1 DP-1\n\tHDR: disabled\n"), Some(false));
        assert_eq!(kscreen_hdr("Output: 1 DP-1\n"), None);
        assert_eq!(
            nvidia_version("NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  595.58.03  Release Build  (dvs-builder@U16)\nGCC version:").as_deref(),
            Some("595.58.03")
        );
        let vk = "GPU0:\n\tdriverName         = radv\n\tdriverInfo         = Mesa 25.2.3-arch1.1\nGPU1:\n\tdriverInfo         = Mesa 25.2.3 (LLVM 20.1.8)\n";
        assert_eq!(vulkan_driver(vk), Some(Gpu::Mesa("25.2.3-arch1.1".into())));
        assert_eq!(vulkan_driver("\tdriverInfo = 570.153.02\n"), Some(Gpu::Nvidia("570.153.02".into())));
        assert!(Gpu::Mesa("25.1.0".into()).native_hdr() && !Gpu::Mesa("24.3.4".into()).native_hdr());
        assert!(Gpu::Nvidia("595.58.03".into()).native_hdr() && !Gpu::Nvidia("570.153.02".into()).native_hdr());
    }

    #[test]
    fn classifies_proton_builds() {
        let r = |p: &str| runner(Path::new(p));
        assert!(r("/home/u/.steam/root/compatibilitytools.d/GE-Proton10-25").wayland);
        assert_eq!(r("/home/u/.steam/root/compatibilitytools.d/GE-Proton10-25/files").name, "GE-Proton10-25");
        assert!(!r("/home/u/.steam/root/compatibilitytools.d/GE-Proton9-27").wayland);
        assert!(r("/usr/share/steam/compatibilitytools.d/proton-cachyos-10.0-20250714-slr").wayland);
        assert!(!r("/lib/steamapps/common/Proton - Experimental").wayland);
        assert!(!r("/home/u/.config/heroic/tools/wine/Wine-GE-Proton8-26/bin/wine").wayland);
    }

    fn probe() -> Probe {
        Probe {
            displays: vec![Display { connector: "DP-1".into(), name: Some("OLED".into()), pq: true, mode: Some((3440, 1440)) }],
            wayland: true,
            desktop: "KDE".into(),
            desktop_version: Some("6.4.5".into()),
            desktop_hdr: Some(true),
            gpu: Some(Gpu::Mesa("25.2.3".into())),
            hdr_layer: false,
            gamescope: false,
        }
    }

    #[test]
    fn plans_from_what_the_system_offers() {
        let ge = Runner { name: "GE-Proton10-25".into(), wayland: true };
        let valve = Runner { name: "Proton 10.0".into(), wayland: false };
        assert!(status(&Probe { displays: vec![Display::default()], ..probe() }, Some(&ge), true).is_none(), "no HDR display");

        let s = status(&probe(), Some(&ge), true).unwrap();
        assert!(s.missing.is_empty(), "{:?}", s.missing);
        assert_eq!(s.plan.unwrap().line(), "PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1");

        let s = status(&Probe { wayland: false, desktop_hdr: Some(false), ..probe() }, Some(&ge), true).unwrap();
        assert!(s.missing[0].starts_with("X11 has no HDR output") && !s.sdr);
        let s = status(&Probe { desktop_hdr: Some(false), ..probe() }, Some(&ge), true).unwrap();
        assert!(s.sdr && s.missing[0].starts_with("Desktop HDR disabled"));
        assert!(!status(&Probe { desktop_hdr: None, ..probe() }, Some(&ge), true).unwrap().sdr, "not checked counts as HDR");

        let old = Probe { gpu: Some(Gpu::Nvidia("570.153.02".into())), ..probe() };
        let s = status(&old, Some(&ge), true).unwrap();
        assert!(s.missing.iter().any(|m| m.contains("NVIDIA 595")), "{:?}", s.missing);
        let s = status(&Probe { hdr_layer: true, ..old }, Some(&ge), true).unwrap();
        assert!(s.missing.is_empty());
        assert!(s.plan.unwrap().line().ends_with("ENABLE_HDR_WSI=1"));

        let s = status(&probe(), Some(&valve), true).unwrap();
        assert!(s.plan.is_none() && s.missing.iter().any(|m| m.contains("GE-Proton 10")));
        let s = status(&Probe { gamescope: true, ..probe() }, Some(&ge), true).unwrap();
        assert_eq!(s.plan.unwrap().line(), "PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1", "Wayland first");
        assert!(s.alt.unwrap().gamescope.is_some(), "gamescope as the second choice");
        assert!(status(&probe(), Some(&ge), true).unwrap().alt.is_none(), "no gamescope installed");
        let s = status(&Probe { gamescope: true, ..probe() }, Some(&valve), true).unwrap();
        assert!(s.alt.is_none());
        assert_eq!(s.plan.unwrap().line(), "DXVK_HDR=1 gamescope -W 3440 -H 1440 -f --hdr-enabled --force-grab-cursor --");
        let s = status(&Probe { gamescope: true, ..probe() }, Some(&valve), false).unwrap();
        assert!(s.plan.is_none(), "gamescope wrapper only through Steam launch options");

        let s = status(&probe(), None, true).unwrap();
        assert!(s.plan.is_none() && s.missing[0].starts_with("Proton build not detected"));
    }

    #[test]
    fn merges_and_strips_launch_options() {
        let wl = Plan { env: vec![("PROTON_ENABLE_WAYLAND", "1"), ("DXVK_HDR", "1")], gamescope: None };
        let gs = Plan { env: vec![("DXVK_HDR", "1")], gamescope: Some("gamescope -W 3840 -H 2160 -f --hdr-enabled --force-grab-cursor --".into()) };
        let mods = "WINEDLLOVERRIDES=\"winmm,version=n,b\" %command% -modded";

        let m = merge(mods, &wl);
        assert_eq!(m, format!("PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 {mods}"));
        assert!(has_plan(&m, &wl) && !has_plan(mods, &wl));
        assert_eq!(merge(&m, &wl), m, "already set: unchanged");
        assert_eq!(strip(&m), mods);

        assert_eq!(merge("", &wl), "PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 %command%");
        assert_eq!(strip("PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 %command%"), "");
        assert_eq!(merge("-skipStartScreen", &wl), "PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 %command% -skipStartScreen");
        assert_eq!(merge("DXVK_HDR=0 %command%", &wl), "PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 %command%");

        let g = merge(mods, &gs);
        assert_eq!(g, "DXVK_HDR=1 WINEDLLOVERRIDES=\"winmm,version=n,b\" gamescope -W 3840 -H 2160 -f --hdr-enabled --force-grab-cursor -- %command% -modded");
        assert!(has_plan(&g, &gs));
        assert_eq!(strip(&g), mods);
        let old = "DXVK_HDR=1 gamescope -W 3840 -H 2160 -f --hdr-enabled -- %command% -modded";
        assert!(!has_plan(old, &gs), "wrapper without the cursor grab needs updating");
        assert_eq!(merge(old, &gs), "DXVK_HDR=1 gamescope -W 3840 -H 2160 -f --hdr-enabled --force-grab-cursor -- %command% -modded");
        assert_eq!(strip("gamescope -f -- %command%"), "gamescope -f -- %command%", "the user's own gamescope stays");
    }
}
