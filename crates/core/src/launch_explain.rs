//! Breaks a Steam launch options line into its parts (variables, wrapper
//! programs, `%command%`, game arguments) and explains each one: what it
//! does, and which Game panel item adds it. Feeds Netrunner's Launch options
//! section. Pure string work, so lines pasted from other launchers or guides
//! are explained the same way.

use serde::Serialize;

use crate::{hdr, linux_setup, optiscaler};

/// What sits where in the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `KEY=value` in front of `%command%`.
    Env,
    /// A program that starts the game (`gamescope … --`, `mangohud`).
    Wrapper,
    /// `%command%` itself.
    Command,
    /// Passed to the game.
    Arg,
}

/// One entry inside a part: a DLL in the override list, a gamescope flag.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub name: String,
    pub detail: String,
}

/// One part of the line, under one bracket.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Part {
    /// Whitespace between the previous part and this one, as typed.
    pub lead: String,
    pub text: String,
    pub kind: Kind,
    /// Short label under the bracket.
    pub label: String,
    pub detail: String,
    pub items: Vec<Item>,
    /// The Game panel item that adds this part, or `None` when CPMX2077
    /// never adds it.
    pub added_by: Option<String>,
    /// Problems with this part where it stands.
    pub warnings: Vec<String>,
    /// In CPMX2077's list of known options.
    pub known: bool,
}

/// Which mod owns a DLL name in the game folder, for the override list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Owners {
    /// The name ReShade is installed as (`dxgi.dll`, `d3d12.dll`).
    pub reshade: Option<String>,
    /// The name OptiScaler loads as (`wininet.dll`, `dxgi.dll`).
    pub optiscaler: Option<String>,
    /// Explaining for Windows: no Wine, so overrides and Proton variables do
    /// nothing.
    pub windows: bool,
}

impl Owners {
    pub fn of_game(game_dir: &std::path::Path) -> Self {
        Owners {
            reshade: crate::reshade::installed_dll(game_dir).map(String::from),
            optiscaler: optiscaler::installed_proxy(game_dir).map(String::from),
            windows: cfg!(windows),
        }
    }
}

const SET_LAUNCH: &str = "Game panel › Steam launch options › Set launch options";
const SET_HDR: &str = "Game panel › HDR › Set HDR launch options";
const SET_FSR4: &str = "Game panel › FSR 4 shader overrides › Set launch option";
const REDMOD_SWITCH: &str = "Game panel › Frameworks › REDmod switch";
const NO_EFFECT_WINDOWS: &str = "Wine/Proton setting: no effect on Windows.";

/// Words of the line with the whitespace before each, splitting on
/// whitespace outside single or double quotes.
fn words(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lead = String::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) => {
                word.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c.is_whitespace() => {
                if word.is_empty() {
                    lead.push(c);
                } else {
                    out.push((std::mem::take(&mut lead), std::mem::take(&mut word)));
                    lead.push(c);
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    quote = Some(c);
                }
                word.push(c);
            }
        }
    }
    if !word.is_empty() {
        out.push((lead, word));
    }
    out
}

/// `KEY` and the unquoted value of a `KEY=value` word.
fn assignment(word: &str) -> Option<(&str, String)> {
    let (k, v) = word.split_once('=')?;
    let ok = !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then(|| (k, v.trim_matches(|c| c == '\'' || c == '"').to_string()))
}

fn part(lead: String, text: String, kind: Kind, label: &str, detail: impl Into<String>) -> Part {
    Part { lead, text, kind, label: label.into(), detail: detail.into(), items: Vec::new(), added_by: None, warnings: Vec::new(), known: true }
}

/// The parts of `line`, in order.
pub fn explain(line: &str, owners: &Owners) -> Vec<Part> {
    let words = words(line.trim());
    let cmd = words.iter().position(|(_, w)| w == "%command%");
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let (lead, w) = words[i].clone();
        let before = cmd.is_some_and(|c| i < c);
        if Some(i) == cmd {
            out.push(part(
                lead,
                w,
                Kind::Command,
                "Game command",
                "Placeholder Steam replaces with the command that starts the game (through Proton on Linux). \
                 Variables and wrapper programs go in front of it, game arguments after it.",
            ));
        } else if before && let Some((key, value)) = assignment(&w) {
            out.push(env_part(lead, w.clone(), key, &value, owners));
        } else if before && w == "gamescope" {
            // The wrapper and its flags up to `--`.
            let end = words[i..cmd.unwrap_or(words.len())].iter().position(|(_, t)| t == "--").map(|e| i + e);
            let last = end.unwrap_or(cmd.unwrap_or(words.len()) - 1);
            out.push(gamescope_part(lead, &words[i..=last], end.is_some(), owners));
            i = last;
        } else if before {
            out.push(wrapper_part(lead, w));
        } else {
            out.push(arg_part(lead, w, cmd.is_none()));
        }
        i += 1;
    }
    out
}

fn env_part(lead: String, text: String, key: &str, value: &str, owners: &Owners) -> Part {
    let wine = !matches!(key, "MANGOHUD" | "ENABLE_HDR_WSI");
    let mut p = match key {
        "WINEDLLOVERRIDES" => {
            let mut p = part(
                lead,
                text,
                Kind::Env,
                "DLL overrides",
                "Wine loads the listed DLLs from the game folder (native) before its own (builtin). \
                 CET, RED4ext, ReShade and OptiScaler load through DLLs with system names; without an override Wine uses its own copy and the mod does not start.",
            );
            p.items = override_items(value, owners);
            p.added_by = Some(SET_LAUNCH.into());
            p
        }
        optiscaler::OVERRIDE_VAR => {
            let mut p = part(
                lead,
                text,
                Kind::Env,
                "FSR 4 shaders",
                format!(
                    "vkd3d-proton replaces the game's FSR 4 shaders with the SPIR-V files in this folder. \
                     They match AMD's upscaler DLL {} only (OptiScaler section). `Z:` is Wine's drive for the Linux root folder.",
                    optiscaler::FSR_VERSION
                ),
            );
            p.added_by = Some(SET_FSR4.into());
            if !value.starts_with("Z:/") {
                p.warnings.push("Value is not a Z:/ path: vkd3d-proton reads the folder through Wine's Z: drive.".into());
            }
            p
        }
        "DXVK_HDR" => {
            let mut p = part(lead, text, Kind::Env, "HDR output", "DXVK/vkd3d-proton report an HDR colour space to the game, so its HDR modes (HDR10 PQ) can be picked. 1 = on.");
            p.added_by = Some(SET_HDR.into());
            p
        }
        "PROTON_ENABLE_WAYLAND" => {
            let mut p = part(
                lead,
                text,
                Kind::Env,
                "Wayland driver",
                "Runs the game through Wine's Wayland driver instead of XWayland, which HDR output needs without gamescope. \
                 GE-Proton 10 or newer and proton-cachyos only; Valve's Proton ignores it.",
            );
            p.added_by = Some(SET_HDR.into());
            p
        }
        "ENABLE_HDR_WSI" => {
            let mut p = part(lead, text, Kind::Env, "HDR layer", "Turns on vk_hdr_layer, the Vulkan layer that adds HDR swapchains on Wayland for drivers older than Mesa 25.1 or NVIDIA 595.");
            p.added_by = Some(SET_HDR.into());
            p
        }
        "PROTON_ENABLE_HDR" => part(lead, text, Kind::Env, "HDR (GE-Proton)", "GE-Proton shorthand that sets DXVK_HDR=1. Not added by CPMX2077, which sets DXVK_HDR directly."),
        "PROTON_LOG" => part(lead, text, Kind::Env, "Proton log", "Writes steam-1091500.log to the home folder. 1 = on. The log grows quickly; remove when done."),
        "PROTON_ENABLE_NVAPI" => part(lead, text, Kind::Env, "NVAPI", "Exposes NVIDIA's NVAPI to the game (DLSS, Reflex). 1 = on. NVIDIA GPUs only."),
        "PROTON_HIDE_NVIDIA_GPU" => part(lead, text, Kind::Env, "Hide NVIDIA GPU", "Reports the NVIDIA GPU as AMD to the game. 1 = on."),
        "PROTON_NO_ESYNC" => part(lead, text, Kind::Env, "esync off", "Turns off eventfd-based synchronisation in Wine. 1 = off. Troubleshooting only."),
        "PROTON_NO_FSYNC" => part(lead, text, Kind::Env, "fsync off", "Turns off futex-based synchronisation in Wine. 1 = off. Troubleshooting only."),
        "PROTON_USE_WINED3D" => part(lead, text, Kind::Env, "WineD3D", "Uses Wine's OpenGL-based Direct3D instead of DXVK/vkd3d-proton. Cyberpunk 2077 is Direct3D 12 and does not run on it."),
        "DXVK_ASYNC" => part(lead, text, Kind::Env, "DXVK async", "Compiles shaders in the background. Read only by dxvk-async builds and older GE-Proton; current DXVK ignores it. Cyberpunk 2077 uses vkd3d-proton (Direct3D 12), not DXVK."),
        "DXVK_FRAME_RATE" => part(lead, text, Kind::Env, "Frame cap", "Frame rate limit applied by DXVK/vkd3d-proton. 0 = no limit."),
        "VKD3D_CONFIG" => part(lead, text, Kind::Env, "vkd3d-proton options", "Comma-separated vkd3d-proton options, for example dxr (ray tracing) or no_upload_hvv."),
        "WINE_FULLSCREEN_FSR" => part(lead, text, Kind::Env, "Wine FSR 1", "Upscales with AMD FSR 1 when the game runs below the display resolution in full screen. Proton-GE builds only."),
        "MANGOHUD" => part(lead, text, Kind::Env, "MangoHud", "Shows the MangoHud overlay (frame rate, frame time, load). 1 = on."),
        _ => {
            let mut p = part(lead, text, Kind::Env, "Variable", format!("Sets the environment variable {key} for the game and Proton. Not in CPMX2077's list of known options."));
            p.known = false;
            return p;
        }
    };
    if owners.windows && wine {
        p.warnings.push(NO_EFFECT_WINDOWS.into());
    }
    p
}

/// What each DLL in a `WINEDLLOVERRIDES` value is for.
fn override_items(value: &str, owners: &Owners) -> Vec<Item> {
    let is = |o: &Option<String>, d: &str| o.as_deref().is_some_and(|n| n.trim_end_matches(".dll").eq_ignore_ascii_case(d));
    let mut out = Vec::new();
    for entry in value.split(';').filter(|e| !e.trim().is_empty()) {
        let (names, mode) = entry.split_once('=').unwrap_or((entry, ""));
        let mode = match mode.trim().to_ascii_lowercase().as_str() {
            "n,b" => "native, then builtin",
            "b,n" => "builtin, then native",
            "n" => "native only",
            "b" => "builtin only",
            "" => "disabled",
            _ => "mode not recognised",
        };
        for d in names.split(',').map(|d| d.trim().trim_end_matches(".dll")).filter(|d| !d.is_empty()) {
            let what = match d.to_ascii_lowercase().as_str() {
                "version" => "CET loader (bin/x64/version.dll).".to_string(),
                "winmm" => "RED4ext loader (bin/x64/winmm.dll).".to_string(),
                "d3dcompiler_47" => "Microsoft's shader compiler, used by ReShade to build effects (Game panel › Shader compiler for ReShade).".to_string(),
                d if is(&owners.reshade, d) && is(&owners.optiscaler, d) => format!("ReShade and OptiScaler, both as {d}.dll."),
                d if is(&owners.reshade, d) => format!("ReShade (bin/x64/{d}.dll)."),
                d if is(&owners.optiscaler, d) => format!("OptiScaler (bin/x64/{d}.dll)."),
                "dxgi" | "d3d12" => format!("ReShade or OptiScaler name; no ReShade or OptiScaler found as {d}.dll in the game folder."),
                "wininet" => "OptiScaler name; OptiScaler not found as wininet.dll in the game folder.".to_string(),
                _ => "Not added by CPMX2077.".to_string(),
            };
            out.push(Item { name: format!("{d} ({mode})"), detail: what });
        }
    }
    out
}

fn gamescope_part(lead: String, words: &[(String, String)], closed: bool, owners: &Owners) -> Part {
    let text = words.iter().enumerate().map(|(i, (l, w))| if i == 0 { w.clone() } else { format!("{l}{w}") }).collect::<String>();
    let hdr = words.iter().any(|(_, w)| w == "--hdr-enabled");
    let mut p = part(
        lead,
        text,
        Kind::Wrapper,
        "gamescope",
        "Valve's micro-compositor: runs the game in its own nested window. CPMX2077 uses it for HDR on Proton builds without Wine's Wayland driver (Valve's Proton).",
    );
    if hdr {
        p.added_by = Some(SET_HDR.into());
    }
    let mut it = words.iter().skip(1).map(|(_, w)| w.as_str()).peekable();
    while let Some(w) = it.next() {
        let with_value = |label: &str, it: &mut std::iter::Peekable<_>| -> Item {
            let v: Option<&str> = it.next();
            Item { name: format!("{w} {}", v.unwrap_or("")).trim().to_string(), detail: label.to_string() }
        };
        let item = match w {
            "-W" | "--output-width" => with_value("Output width in pixels.", &mut it),
            "-H" | "--output-height" => with_value("Output height in pixels.", &mut it),
            "-w" | "--nested-width" => with_value("Width the game renders at.", &mut it),
            "-h" | "--nested-height" => with_value("Height the game renders at.", &mut it),
            "-r" | "--nested-refresh" => with_value("Refresh rate in Hz.", &mut it),
            "-f" | "--fullscreen" => Item { name: w.into(), detail: "Starts full screen.".into() },
            "-b" | "--borderless" => Item { name: w.into(), detail: "Starts as a borderless window.".into() },
            "--hdr-enabled" => Item { name: w.into(), detail: "Outputs HDR10 to the display.".into() },
            hdr::GRAB_CURSOR => Item {
                name: w.into(),
                detail: "Keeps the mouse captured; without it the camera stops turning at the window edge.".into(),
            },
            "--" => Item { name: w.into(), detail: "End of gamescope's options; what follows is the command it runs.".into() },
            _ => Item { name: w.into(), detail: "gamescope option not in CPMX2077's list.".into() },
        };
        p.items.push(item);
    }
    if !closed {
        p.warnings.push("No -- before %command%: gamescope may read the game command as its own options.".into());
    }
    if owners.windows {
        p.warnings.push("gamescope is Linux only.".into());
    }
    p
}

fn wrapper_part(lead: String, w: String) -> Part {
    match w.as_str() {
        "mangohud" => part(lead, w, Kind::Wrapper, "MangoHud", "Starts the game with the MangoHud overlay (frame rate, frame time, load)."),
        "gamemoderun" => part(lead, w, Kind::Wrapper, "GameMode", "Starts the game under Feral GameMode, which sets the CPU governor and process priority while it runs."),
        "prime-run" => part(lead, w, Kind::Wrapper, "NVIDIA PRIME", "Runs the game on the NVIDIA GPU of a hybrid-graphics laptop."),
        "--" => part(lead, w, Kind::Wrapper, "End of options", "Ends the previous program's options; what follows is the command it runs."),
        _ => {
            let mut p = part(lead, w, Kind::Wrapper, "Program", "Program in front of %command%: Steam runs it with the game command as its arguments. Not in CPMX2077's list of known options.");
            p.known = false;
            p
        }
    }
}

fn arg_part(lead: String, w: String, no_command: bool) -> Part {
    let mut p = if w.eq_ignore_ascii_case(linux_setup::MODDED) {
        let mut p = part(
            lead,
            w,
            Kind::Arg,
            "REDmod mods",
            "The game deploys REDmod mods (folders in mods/) at start and loads them. Replaces the REDprelauncher's Enable mods setting.",
        );
        p.added_by = Some(REDMOD_SWITCH.into());
        p
    } else if w.eq_ignore_ascii_case(linux_setup::LAUNCHER_SKIP) {
        let mut p = part(
            lead,
            w,
            Kind::Arg,
            "Skip launcher",
            "Starts the game without CD Projekt's REDprelauncher window. Its one setting mods need, Enable mods, is -modded.",
        );
        p.added_by = Some(SET_LAUNCH.into());
        p
    } else if w.eq_ignore_ascii_case("-skipStartScreen") {
        part(lead, w, Kind::Arg, "Skip start screen", "Skips the \"press any key\" breach screen before the main menu.")
    } else {
        let mut p = part(lead, w, Kind::Arg, "Game argument", "Passed to the game. Not in CPMX2077's list of known options.");
        p.known = false;
        p
    };
    if let Some((key, _)) = assignment(&p.text) {
        // Steam appends everything after %command% (or the whole line
        // without one) to the game command.
        p.kind = Kind::Env;
        p.label = key.to_string();
        p.detail = format!("Environment variable {key}, placed where Steam passes it to the game as an argument.");
        p.warnings.push(if no_command {
            format!("No %command% in the line: Steam passes {key}=… to the game as an argument and does not set the variable.")
        } else {
            format!("After %command%: Steam passes {key}=… to the game as an argument and does not set the variable. Move it in front of %command%.")
        });
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux() -> Owners {
        Owners::default()
    }

    #[test]
    fn splits_and_labels_the_line_the_app_writes() {
        let line = r#"WINEDLLOVERRIDES="winmm,version=n,b" %command% --launcher-skip -modded"#;
        let parts = explain(line, &linux());
        let kinds: Vec<Kind> = parts.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [Kind::Env, Kind::Command, Kind::Arg, Kind::Arg]);
        assert_eq!(parts[0].text, r#"WINEDLLOVERRIDES="winmm,version=n,b""#);
        assert_eq!(parts[0].items.len(), 2);
        assert!(parts[0].items[0].name.starts_with("winmm (native, then builtin)"));
        assert!(parts[0].items[0].detail.contains("RED4ext"));
        assert!(parts[0].items[1].detail.contains("CET"));
        assert_eq!(parts[2].added_by.as_deref(), Some(SET_LAUNCH));
        assert_eq!(parts[3].added_by.as_deref(), Some(REDMOD_SWITCH));
        // Joining the parts gives back the line.
        assert_eq!(parts.iter().map(|p| format!("{}{}", p.lead, p.text)).collect::<String>(), line);
        assert!(parts.iter().all(|p| p.warnings.is_empty() && p.known));
    }

    #[test]
    fn names_the_mod_behind_each_dll() {
        let owners = Owners { reshade: Some("dxgi.dll".into()), optiscaler: Some("wininet.dll".into()), windows: false };
        let parts = explain(r#"WINEDLLOVERRIDES="winmm,version,dxgi,d3dcompiler_47,wininet=n,b;foo=" %command%"#, &owners);
        let d: Vec<&str> = parts[0].items.iter().map(|i| i.detail.as_str()).collect();
        assert!(d[2].starts_with("ReShade"));
        assert!(d[3].contains("shader compiler"));
        assert!(d[4].starts_with("OptiScaler"));
        assert_eq!(parts[0].items[5].name, "foo (disabled)");
        assert_eq!(d[5], "Not added by CPMX2077.");
        let none = explain(r#"WINEDLLOVERRIDES="dxgi=n,b" %command%"#, &linux());
        assert!(none[0].items[0].detail.contains("no ReShade or OptiScaler found"));
    }

    #[test]
    fn keeps_the_gamescope_wrapper_together() {
        let line = "DXVK_HDR=1 gamescope -W 3840 -H 2160 -f --hdr-enabled --force-grab-cursor -- %command%";
        let parts = explain(line, &linux());
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1].text, "gamescope -W 3840 -H 2160 -f --hdr-enabled --force-grab-cursor --");
        assert_eq!(parts[1].added_by.as_deref(), Some(SET_HDR));
        let names: Vec<&str> = parts[1].items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["-W 3840", "-H 2160", "-f", "--hdr-enabled", "--force-grab-cursor", "--"]);
        assert!(parts[1].warnings.is_empty());
        let open = explain("gamescope -f %command%", &linux());
        assert_eq!(open[0].text, "gamescope -f");
        assert_eq!(open[0].warnings.len(), 1);
    }

    #[test]
    fn explains_the_other_app_options() {
        let line = "PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1 ENABLE_HDR_WSI=1 VKD3D_SHADER_OVERRIDE='Z:/home/v/.local/share/cpmx/fsr4 x' mangohud %command%";
        let parts = explain(line, &linux());
        let labels: Vec<&str> = parts.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Wayland driver", "HDR output", "HDR layer", "FSR 4 shaders", "MangoHud", "Game command"]);
        assert_eq!(parts[3].text, "VKD3D_SHADER_OVERRIDE='Z:/home/v/.local/share/cpmx/fsr4 x'");
        assert_eq!(parts[3].added_by.as_deref(), Some(SET_FSR4));
        assert!(parts[3].warnings.is_empty());
        assert!(parts[4].added_by.is_none());
    }

    #[test]
    fn flags_unknown_and_misplaced_parts() {
        let parts = explain("FOO=1 -windowed", &linux());
        assert_eq!(parts[0].kind, Kind::Env);
        assert!(parts[0].warnings[0].contains("No %command%"));
        assert!(!parts[1].known);
        let after = explain("%command% WINEDLLOVERRIDES=x", &linux());
        assert!(after[1].warnings[0].starts_with("After %command%"));
        let tool = explain("obs-gamecapture %command%", &linux());
        assert_eq!(tool[0].kind, Kind::Wrapper);
        assert!(!tool[0].known);
        assert!(explain("   ", &linux()).is_empty());
    }

    #[test]
    fn marks_wine_settings_on_windows() {
        let owners = Owners { windows: true, ..Owners::default() };
        let parts = explain(r#"WINEDLLOVERRIDES="winmm=n,b" %command% -modded"#, &owners);
        assert_eq!(parts[0].warnings, [NO_EFFECT_WINDOWS]);
        assert!(parts[2].warnings.is_empty());
    }
}
