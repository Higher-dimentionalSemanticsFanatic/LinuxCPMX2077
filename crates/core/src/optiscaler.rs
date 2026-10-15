//! OptiScaler: the upscaler swapper from github.com/optiscaler/OptiScaler,
//! installed as a tracked mod, plus the two things FSR 4.1.1 shader
//! overrides need under Proton:
//!
//! - AMD's `amd_fidelityfx_upscaler_dx12.dll` 4.1.1.2740, taken from AMD's
//!   FidelityFX SDK repository at a pinned commit and checked against its git
//!   blob id and version resource. It goes into the OptiScaler mod in place of
//!   the upscaler DLL OptiScaler ships.
//! - The prebuilt SPIR-V overrides from zgauthier2000's
//!   fsr4-shader-performance-overrides repository, downloaded file by file
//!   (each checked against its git blob id) into the app's data folder, and
//!   handed to vkd3d-proton with `VKD3D_SHADER_OVERRIDE` in the launch options
//!   (see `linux_setup`). Those files match AMD's 4.1.1.2740 shaders only.
//!
//! OptiScaler loads as a system DLL name next to the game exe. For Cyberpunk
//! 2077 its wiki lists `dxgi.dll` and `wininet.dll`; `winmm.dll` and
//! `version.dll` belong to RED4ext and CET, and ReShade usually has
//! `dxgi.dll`, so `wininet.dll` comes first.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::activity::{self, Kind};
use crate::db::{Db, GameRow, ModRow, NewMod};
use crate::install::{InstallOptions, InstallReport, Installer, resolve_ci};
use crate::sources::github::{self, Asset, Release, TreeBlob};
use crate::{Error, Result};

pub const OWNER: &str = "optiscaler";
pub const REPO: &str = "OptiScaler";
/// `mods.source` of OptiScaler; `source_file` is the DLL name it loads as.
pub const SOURCE: &str = "optiscaler";
pub const MOD_NAME: &str = "OptiScaler";
pub const BIN: &str = "bin/x64";
/// Names OptiScaler loads under in Cyberpunk 2077 (OptiScaler wiki,
/// "Cyberpunk 2077"), in order of preference.
pub const PROXY_NAMES: &[&str] = &["wininet.dll", "dxgi.dll"];
/// OptiScaler's own DLL in its release archive (`nvngx.dll` before 0.7).
const MAIN_DLLS: &[&str] = &["optiscaler.dll", "nvngx.dll"];
pub const INI: &str = "OptiScaler.ini";
/// Setup scripts in the archive; the app does their job itself.
const SCRIPT_EXTS: &[&str] = &["bat", "cmd", "sh", "ps1"];

/// AMD's FSR upscaler DLL, the one the shader overrides are built from.
pub const UPSCALER_DLL: &str = "amd_fidelityfx_upscaler_dx12.dll";
pub const FSR_VERSION: &str = "4.1.1.2740";
pub const AMD_OWNER: &str = "GPUOpen-LibrariesAndSDKs";
pub const AMD_REPO: &str = "FidelityFX-SDK";
/// FidelityFX SDK 2.3.0, as named in the overrides' `prebuilt/NOTICE.md`.
pub const AMD_COMMIT: &str = "60f4ea81909200d8542eca14dccb2628b763a9a3";
pub const AMD_PATH: &str = "Kits/FidelityFX/signedbin/amd_fidelityfx_upscaler_dx12.dll";
pub const AMD_BLOB: &str = "199de3a500a1d153b7e73fa5ca0adbd2a4ac1229";
const MAX_AMD_DLL: u64 = 64 << 20;

/// The release asset to install: the one archive whose name names
/// OptiScaler, or the only archive.
pub fn pick_asset(release: &Release) -> Option<&Asset> {
    let archives: Vec<&Asset> = release.assets.iter().filter(|a| a.installable).collect();
    archives
        .iter()
        .copied()
        .find(|a| a.name.to_ascii_lowercase().contains("optiscaler"))
        .or_else(|| (archives.len() == 1).then(|| archives[0]))
}

/// The newest stable release with an archive to install.
pub fn latest_stable(releases: &[Release]) -> Option<&Release> {
    releases.iter().find(|r| !r.prerelease && pick_asset(r).is_some())
}

fn file_name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Where each archive file goes: everything next to OptiScaler's DLL (and
/// below it) into `bin/x64`, the DLL itself renamed to `proxy`. Setup
/// scripts are left out.
pub fn layout(files: &[String], proxy: &str) -> Result<Vec<(String, String)>> {
    let main = files
        .iter()
        .filter(|f| MAIN_DLLS.contains(&file_name_of(f).to_ascii_lowercase().as_str()))
        .min_by_key(|f| (f.matches('/').count(), !file_name_of(f).eq_ignore_ascii_case("optiscaler.dll")))
        .ok_or_else(|| Error::Other("the archive has no OptiScaler.dll".into()))?;
    let root = main.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
    let mut out = Vec::new();
    for f in files.iter().filter(|f| f.starts_with(&root)) {
        let rel = &f[root.len()..];
        if SCRIPT_EXTS.contains(&ext_of(rel).as_str()) {
            continue;
        }
        let target = if f == main { proxy.to_string() } else { rel.to_string() };
        out.push((f.clone(), format!("{BIN}/{target}")));
    }
    Ok(out)
}

fn bin_file(game_dir: &Path, name: &str) -> PathBuf {
    resolve_ci(game_dir, &format!("{BIN}/{name}"))
}

/// OptiScaler installed by the app (the newest, if several).
pub fn installed_mod(db: &Db, game_id: i64) -> Result<Option<ModRow>> {
    Ok(db.mods(game_id)?.into_iter().filter(|m| m.source == SOURCE).max_by_key(|m| m.id))
}

fn mod_proxy(db: &Db, mod_id: i64) -> Result<Option<String>> {
    Ok(db.mod_files(mod_id)?.into_iter().find_map(|f| {
        let name = file_name_of(&f.rel_path).to_ascii_lowercase();
        (f.rel_path.matches('/').count() == 2 && PROXY_NAMES.contains(&name.as_str())).then_some(name)
    }))
}

/// The name OptiScaler loads as in the game folder: `OptiScaler.ini` is there
/// and one of [`PROXY_NAMES`] is a file that isn't ReShade. Disk only, so the
/// launch options can be worked out without the database.
pub fn installed_proxy(game_dir: &Path) -> Option<&'static str> {
    if !bin_file(game_dir, INI).is_file() {
        return None;
    }
    PROXY_NAMES.iter().copied().find(|n| {
        let p = bin_file(game_dir, n);
        p.is_file() && !crate::reshade::is_reshade_dll(&p)
    })
}

/// The name to install OptiScaler under: `wanted` if given, else the name the
/// installed copy uses, else the first free name.
pub fn pick_proxy(db: &Db, game: &GameRow, wanted: Option<&str>) -> Result<String> {
    let game_dir = Path::new(&game.path);
    let ours = installed_mod(db, game.id)?;
    let usable = |name: &str| -> Result<()> {
        if !bin_file(game_dir, name).exists() {
            return Ok(());
        }
        let rel = format!("{BIN}/{name}");
        let owner = db.owners_of(game.id, &rel, -1)?.first().map(|f| f.mod_id);
        match owner {
            Some(id) if ours.as_ref().is_some_and(|m| m.id == id) => Ok(()),
            Some(id) => Err(Error::Conflict(format!("{rel} belongs to {}", db.get_mod(id)?.name))),
            None => Err(Error::Conflict(format!("{rel} is already there and not from a mod in CPMX2077"))),
        }
    };
    if let Some(w) = wanted {
        let w = w.to_ascii_lowercase();
        if !PROXY_NAMES.contains(&w.as_str()) {
            return Err(Error::Other(format!("OptiScaler loads in Cyberpunk 2077 as {}", PROXY_NAMES.join(" or "))));
        }
        usable(&w)?;
        return Ok(w);
    }
    if let Some(m) = &ours
        && let Some(name) = mod_proxy(db, m.id)?
    {
        return Ok(name);
    }
    let mut taken = Vec::new();
    for name in PROXY_NAMES {
        match usable(name) {
            Ok(()) => return Ok(name.to_string()),
            Err(Error::Conflict(why)) => taken.push(why),
            Err(e) => return Err(e),
        }
    }
    Err(Error::Conflict(taken.join("; ")))
}

/// File version and product name from a 64-bit DLL's version resource.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DllVersion {
    pub version: Option<String>,
    pub product: Option<String>,
}

pub fn dll_version(bytes: &[u8]) -> Option<DllVersion> {
    use pelite::pe64::{Pe, PeFile};
    let file = PeFile::from_bytes(bytes).ok()?;
    let vi = file.resources().ok()?.version_info().ok()?;
    let version = vi.fixed().map(|f| {
        let v = f.dwFileVersion;
        format!("{}.{}.{}.{}", v.Major, v.Minor, v.Patch, v.Build)
    });
    let lang = vi.translation().first().copied();
    let product = lang.and_then(|l| vi.value(l, "ProductName")).map(|s| s.trim().to_string());
    Some(DllVersion { version, product })
}

/// Check AMD's upscaler DLL: the pinned git blob, and the version resource
/// naming 4.1.1.2740.
pub fn check_amd_dll(bytes: &[u8]) -> Result<()> {
    let blob = github::git_blob_sha(bytes);
    if blob != AMD_BLOB {
        return Err(Error::Integrity(format!("{UPSCALER_DLL}: git blob {blob} is not AMD's {AMD_BLOB}")));
    }
    match dll_version(bytes).and_then(|v| v.version) {
        Some(v) if v == FSR_VERSION => Ok(()),
        v => Err(Error::Integrity(format!("{UPSCALER_DLL}: version {} is not {FSR_VERSION}", v.unwrap_or_else(|| "unknown".into())))),
    }
}

/// Download AMD's upscaler DLL 4.1.1.2740 from the FidelityFX SDK repository
/// into `dest_dir` (reused when a checked copy is already there).
pub fn download_amd_dll(gh: &github::Client, dest_dir: &Path) -> Result<PathBuf> {
    let path = dest_dir.join(format!("{UPSCALER_DLL}.{FSR_VERSION}"));
    if let Ok(b) = std::fs::read(&path) {
        if check_amd_dll(&b).is_ok() {
            return Ok(path);
        }
        let _ = std::fs::remove_file(&path);
    }
    activity::record(Kind::Download, format!("Downloading {UPSCALER_DLL} {FSR_VERSION} from {AMD_OWNER}/{AMD_REPO} at {}", &AMD_COMMIT[..12]));
    let bytes = gh.raw_file(AMD_OWNER, AMD_REPO, AMD_COMMIT, AMD_PATH, AMD_BLOB, MAX_AMD_DLL)?;
    check_amd_dll(&bytes)?;
    std::fs::create_dir_all(dest_dir)?;
    std::fs::write(&path, &bytes)?;
    activity::record_path(Kind::Verify, format!("{UPSCALER_DLL}: git blob {AMD_BLOB} and version {FSR_VERSION} match"), &path);
    Ok(path)
}

/// `OptiScaler.ini` settings that make it start on FSR 4 (OptiScaler's
/// sample ini): the FFX upscaler for DirectX 12, with the FSR 4 backend.
/// On `auto` it picks DLSS or FSR 4 only on GPUs it considers capable and
/// XeSS otherwise, and the FSR 3 backend on anything but RDNA4.
pub const FSR4_INI: &[(&str, &str, &str)] = &[("Upscalers", "Dx12Upscaler", "ffx"), ("FSR", "UpscalerIndex", "0")];

/// `ini` with [`FSR4_INI`] set; everything else stays.
pub fn fsr4_ini(ini: &str) -> String {
    FSR4_INI.iter().fold(ini.to_string(), |acc, (section, key, value)| crate::reshade::set_ini_value(&acc, section, key, value))
}

/// Install (or update) OptiScaler from its release `archive` as `proxy` (or
/// the name [`pick_proxy`] chooses). With `amd_dll`, AMD's checked upscaler
/// DLL replaces the one in the archive. The previous OptiScaler is replaced.
pub fn install(inst: &Installer, game: &GameRow, archive: &Path, tag: &str, proxy: Option<&str>, amd_dll: Option<&Path>) -> Result<InstallReport> {
    let proxy = pick_proxy(inst.db, game, proxy)?;
    let mut p = inst.prepare(game, archive)?;
    p.fomod = None;
    let staged = (|| -> Result<Vec<String>> {
        let plan = layout(&p.files, &proxy)?;
        let relaid = p.dir.join("relaid");
        let mut targets = Vec::new();
        for (from, to) in &plan {
            let dst = relaid.join(to);
            std::fs::create_dir_all(dst.parent().unwrap_or(&relaid))?;
            std::fs::rename(p.dir.join("files").join(from), dst)?;
            targets.push(to.clone());
        }
        if let Some(amd) = amd_dll {
            let bytes = std::fs::read(amd)?;
            check_amd_dll(&bytes)?;
            let rel = format!("{BIN}/{UPSCALER_DLL}");
            // Replace whatever spelling the archive used.
            targets.retain(|t| {
                let same = t.eq_ignore_ascii_case(&rel);
                if same {
                    let _ = std::fs::remove_file(relaid.join(t));
                }
                !same
            });
            let dst = relaid.join(&rel);
            std::fs::create_dir_all(dst.parent().unwrap_or(&relaid))?;
            std::fs::write(dst, bytes)?;
            targets.push(rel);
            // Start on FSR 4 instead of OptiScaler's own pick.
            let ini_rel = format!("{BIN}/{INI}");
            let ini_path = match targets.iter().find(|t| t.eq_ignore_ascii_case(&ini_rel)) {
                Some(t) => relaid.join(t),
                None => {
                    targets.push(ini_rel.clone());
                    relaid.join(&ini_rel)
                }
            };
            let ini = std::fs::read(&ini_path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            std::fs::write(&ini_path, fsr4_ini(&ini))?;
        }
        std::fs::remove_dir_all(p.dir.join("files"))?;
        std::fs::rename(&relaid, p.dir.join("files"))?;
        Ok(targets)
    })();
    match staged {
        Ok(t) => {
            p.file_count = t.len();
            p.files = t;
        }
        Err(e) => {
            inst.discard(&p);
            return Err(e);
        }
    }
    let version = match amd_dll {
        Some(_) => format!("{tag} + FSR {FSR_VERSION}"),
        None => tag.to_string(),
    };
    let meta = NewMod {
        name: MOD_NAME.into(),
        version: Some(version),
        source: SOURCE.into(),
        archive_name: p.archive_name.clone(),
        category: Some("Upscaler".into()),
        source_ref: Some(format!("{OWNER}/{REPO}")),
        source_file: Some(proxy),
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

/// OptiScaler in one game, for the OptiScaler page.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub installed: Option<Installed>,
    /// `OptiScaler.ini` is in bin/x64 but no OptiScaler mod is installed.
    pub unmanaged: bool,
    /// The name a new install would use, or why none is free.
    pub proxy_choice: std::result::Result<String, String>,
    /// The upscaler DLL in bin/x64, from its version resource.
    pub upscaler: Option<DllVersion>,
    /// That DLL is AMD's 4.1.1.2740, the one the shader overrides match.
    pub upscaler_matches: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Installed {
    pub mod_id: i64,
    pub version: Option<String>,
    pub proxy: Option<String>,
    pub enabled: bool,
    pub archive_sha256: String,
}

pub fn status(db: &Db, game: &GameRow) -> Result<Status> {
    let installed = match installed_mod(db, game.id)? {
        Some(m) => Some(Installed {
            mod_id: m.id,
            version: m.version.clone(),
            proxy: mod_proxy(db, m.id)?,
            enabled: m.enabled(),
            archive_sha256: m.archive_sha256.clone(),
        }),
        None => None,
    };
    let game_dir = Path::new(&game.path);
    let unmanaged = installed.is_none() && bin_file(game_dir, INI).is_file();
    let proxy_choice = pick_proxy(db, game, None).map_err(|e| match e {
        Error::Conflict(why) => why,
        e => e.to_string(),
    });
    let upscaler = std::fs::read(bin_file(game_dir, UPSCALER_DLL)).ok().and_then(|b| dll_version(&b));
    let upscaler_matches = upscaler.as_ref().and_then(|u| u.version.as_deref()) == Some(FSR_VERSION);
    Ok(Status { installed, unmanaged, proxy_choice, upscaler, upscaler_matches })
}

// ---- FSR 4.1.1 shader overrides ----------------------------------------------

pub const OVERRIDES_OWNER: &str = "zgauthier2000";
pub const OVERRIDES_REPO: &str = "fsr4-shader-performance-overrides";
/// The launch-option variable vkd3d-proton reads replacement shaders from.
pub const OVERRIDE_VAR: &str = "VKD3D_SHADER_OVERRIDE";
const INFO_FILE: &str = "info.json";
const MAX_SHADER: u64 = 4 << 20;
const MAX_SHADERS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// AMD's picture, byte for byte (`prebuilt/`).
    Exact,
    /// Faster, a different picture (`prebuilt-lossy/`).
    Lossy,
}

impl Variant {
    pub fn folder(self) -> &'static str {
        match self {
            Variant::Exact => "prebuilt",
            Variant::Lossy => "prebuilt-lossy",
        }
    }
}

/// The downloaded overrides: commit and file counts per variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverridesInfo {
    pub commit: String,
    pub date: Option<i64>,
    pub exact: usize,
    pub lossy: usize,
}

/// The `.spv` files of both variants in a tree listing (top level of each
/// folder only).
pub fn override_blobs(tree: &[TreeBlob]) -> Vec<&TreeBlob> {
    tree.iter()
        .filter(|b| {
            [Variant::Exact, Variant::Lossy].iter().any(|v| {
                b.path.strip_prefix(v.folder()).and_then(|r| r.strip_prefix('/')).is_some_and(|name| {
                    !name.contains('/') && ext_of(name) == "spv" && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                })
            })
        })
        .collect()
}

pub fn overrides_info(root: &Path) -> Option<OverridesInfo> {
    serde_json::from_slice(&std::fs::read(root.join(INFO_FILE)).ok()?).ok()
}

/// Download both variants at the repository's newest commit into `root`
/// (`root/prebuilt`, `root/prebuilt-lossy`), replacing what was there.
pub fn download_overrides(gh: &github::Client, root: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<OverridesInfo> {
    let commit = gh.latest_commit(OVERRIDES_OWNER, OVERRIDES_REPO, None)?;
    let tree = gh.tree_blobs(OVERRIDES_OWNER, OVERRIDES_REPO, &commit.sha)?;
    let blobs = override_blobs(&tree);
    if blobs.is_empty() || blobs.len() > MAX_SHADERS {
        return Err(Error::Other(format!("{OVERRIDES_OWNER}/{OVERRIDES_REPO}: {} shader files at {}; expected prebuilt/*.spv", blobs.len(), &commit.sha[..7])));
    }
    std::fs::create_dir_all(root)?;
    let tmp = tempfile::Builder::new().prefix("incoming-").tempdir_in(root)?;
    let total = blobs.len() as u64;
    activity::record(Kind::Download, format!("Downloading {total} shader overrides from {OVERRIDES_OWNER}/{OVERRIDES_REPO} at {}", &commit.sha[..12]));
    for (i, b) in blobs.iter().enumerate() {
        let bytes = gh.raw_file(OVERRIDES_OWNER, OVERRIDES_REPO, &commit.sha, &b.path, &b.sha, MAX_SHADER)?;
        let dst = tmp.path().join(&b.path);
        std::fs::create_dir_all(dst.parent().unwrap_or(tmp.path()))?;
        std::fs::write(dst, bytes)?;
        progress(i as u64 + 1, total);
    }
    let count = |v: Variant| blobs.iter().filter(|b| b.path.starts_with(&format!("{}/", v.folder()))).count();
    let info = OverridesInfo { commit: commit.sha.clone(), date: commit.date, exact: count(Variant::Exact), lossy: count(Variant::Lossy) };
    for v in [Variant::Exact, Variant::Lossy] {
        let dst = root.join(v.folder());
        if dst.exists() {
            std::fs::remove_dir_all(&dst)?;
        }
        let src = tmp.path().join(v.folder());
        if src.is_dir() {
            std::fs::rename(src, dst)?;
        }
    }
    std::fs::write(root.join(INFO_FILE), serde_json::to_vec_pretty(&info)?)?;
    activity::record_path(Kind::Verify, format!("{} exact and {} lossy shader overrides; every git blob id matches", info.exact, info.lossy), root);
    Ok(info)
}

/// The variant the user picked for a game, with the folder it points to.
/// Kept in the game's setup folder so the launch-options check reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub variant: Variant,
    pub folder: PathBuf,
}

const CHOICE_FILE: &str = "fsr4-overrides.json";

pub fn load_choice(state_dir: &Path) -> Option<Choice> {
    serde_json::from_slice(&std::fs::read(state_dir.join(CHOICE_FILE)).ok()?).ok()
}

/// Pick a variant (its folder under `root` must hold the downloaded files),
/// or `None` to stop using the overrides.
pub fn save_choice(state_dir: &Path, root: &Path, variant: Option<Variant>) -> Result<()> {
    let path = state_dir.join(CHOICE_FILE);
    let Some(variant) = variant else {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    };
    let folder = root.join(variant.folder());
    if !folder.is_dir() {
        return Err(Error::Other("download the shader overrides first".into()));
    }
    std::fs::create_dir_all(state_dir)?;
    std::fs::write(path, serde_json::to_vec_pretty(&Choice { variant, folder })?)?;
    Ok(())
}

/// The variable's value as Proton sees the folder: `Z:` is the Linux root.
/// `None` for a path a single-quoted launch option can't carry.
pub fn override_value(folder: &Path) -> Option<String> {
    let s = folder.to_str()?;
    (s.starts_with('/') && !s.contains('\'') && !s.contains('\n')).then(|| format!("Z:{s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, pre: bool, names: &[&str]) -> Release {
        Release {
            id: 1,
            tag: tag.into(),
            name: None,
            published: None,
            prerelease: pre,
            notes: String::new(),
            url: String::new(),
            assets: names
                .iter()
                .enumerate()
                .map(|(i, n)| Asset {
                    id: i as i64,
                    name: n.to_string(),
                    size: 1,
                    digest: None,
                    download_url: String::new(),
                    download_count: None,
                    installable: n.ends_with(".7z") || n.ends_with(".zip"),
                })
                .collect(),
        }
    }

    #[test]
    fn picks_the_stable_archive() {
        let rs = vec![
            release("nightly", true, &["OptiScaler_nightly.7z"]),
            release("v0.9.1", false, &["OptiScaler_0.9.1.7z", "Source code.txt"]),
        ];
        let r = latest_stable(&rs).unwrap();
        assert_eq!(r.tag, "v0.9.1");
        assert_eq!(pick_asset(r).unwrap().name, "OptiScaler_0.9.1.7z");
        assert!(pick_asset(&release("x", false, &["a.7z", "b.zip"])).is_none(), "two unnamed archives: none");
        assert_eq!(pick_asset(&release("x", false, &["files.zip"])).unwrap().name, "files.zip");
    }

    #[test]
    fn lays_out_next_to_the_exe() {
        let files: Vec<String> = [
            "OptiScaler_0.9/OptiScaler.dll",
            "OptiScaler_0.9/OptiScaler.ini",
            "OptiScaler_0.9/amd_fidelityfx_upscaler_dx12.dll",
            "OptiScaler_0.9/D3D12_Optiscaler/D3D12Core.dll",
            "OptiScaler_0.9/OptiScaler Setup.bat",
            "OptiScaler_0.9/setup_linux.sh",
            "Readme.txt",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let l = layout(&files, "wininet.dll").unwrap();
        let targets: Vec<&str> = l.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(
            targets,
            [
                "bin/x64/wininet.dll",
                "bin/x64/OptiScaler.ini",
                "bin/x64/amd_fidelityfx_upscaler_dx12.dll",
                "bin/x64/D3D12_Optiscaler/D3D12Core.dll",
            ]
        );
        assert!(layout(&["a/b.dll".to_string()], "wininet.dll").is_err());
    }

    fn opti_zip(dir: &Path, name: &str, ini: &[u8]) -> PathBuf {
        use std::io::Write;
        let path = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (n, data) in [
            ("OptiScaler/OptiScaler.dll", &b"MZ optiscaler"[..]),
            ("OptiScaler/OptiScaler.ini", ini),
            ("OptiScaler/amd_fidelityfx_upscaler_dx12.dll", b"MZ bundled"),
            ("OptiScaler/OptiScaler Setup.bat", b"@echo off"),
        ] {
            z.start_file(n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        path
    }

    #[test]
    fn installs_updates_and_uninstalls() {
        let (dir, db, game) = crate::reshade::tests::setup_game();
        let inst = crate::reshade::tests::installer(&db, dir.path());
        let gdir = Path::new(&game.path);
        let v1 = opti_zip(dir.path(), "OptiScaler_0.9.0.zip", b"[Upscalers]");
        install(&inst, &game, &v1, "v0.9.0", None, None).unwrap();
        assert!(gdir.join("bin/x64/wininet.dll").is_file());
        assert!(gdir.join("bin/x64/OptiScaler.ini").is_file());
        assert!(!gdir.join("bin/x64/OptiScaler Setup.bat").exists(), "setup scripts stay out");
        assert_eq!(installed_proxy(gdir), Some("wininet.dll"));
        let s = status(&db, &game).unwrap();
        let i = s.installed.unwrap();
        assert_eq!((i.version.as_deref(), i.proxy.as_deref()), (Some("v0.9.0"), Some("wininet.dll")));
        assert_eq!(s.proxy_choice.as_deref(), Ok("wininet.dll"), "its own name stays usable");
        assert!(!s.upscaler_matches);

        // An update replaces the mod; a fake AMD DLL is refused before anything changes.
        let v2 = opti_zip(dir.path(), "OptiScaler_0.9.1.zip", b"[Upscalers]\n");
        let fake = dir.path().join("fake.dll");
        std::fs::write(&fake, b"MZ fake").unwrap();
        assert!(matches!(install(&inst, &game, &v2, "v0.9.1", None, Some(&fake)), Err(Error::Integrity(_))));
        assert_eq!(installed_mod(&db, game.id).unwrap().unwrap().version.as_deref(), Some("v0.9.0"));
        install(&inst, &game, &v2, "v0.9.1", None, None).unwrap();
        let mods: Vec<_> = db.mods(game.id).unwrap().into_iter().filter(|m| m.source == SOURCE).collect();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].version.as_deref(), Some("v0.9.1"));

        // dxgi.dll taken by a file no mod owns.
        std::fs::write(gdir.join("bin/x64/dxgi.dll"), crate::reshade::tests::STAND_IN).unwrap();
        assert!(matches!(pick_proxy(&db, &game, Some("dxgi.dll")), Err(Error::Conflict(_))));
        assert!(pick_proxy(&db, &game, Some("winmm.dll")).is_err(), "RED4ext's name is not offered");

        inst.uninstall(mods[0].id).unwrap();
        assert!(!gdir.join("bin/x64/wininet.dll").exists());
        assert_eq!(installed_proxy(gdir), None);
    }

    #[test]
    fn sets_fsr4_in_the_ini() {
        let ini = "; c\r\n[Upscalers]\r\nDx11Upscaler=auto\r\nDx12Upscaler=auto\r\n\r\n[FSR]\r\nDebugView=auto\r\nUpscalerIndex=auto\r\n";
        let out = fsr4_ini(ini);
        assert!(out.contains("Dx12Upscaler=ffx\r\n") && out.contains("UpscalerIndex=0\r\n") && out.contains("Dx11Upscaler=auto"), "{out}");
        let empty = fsr4_ini("");
        assert!(empty.contains("[Upscalers]") && empty.contains("Dx12Upscaler=ffx") && empty.contains("[FSR]") && empty.contains("UpscalerIndex=0"), "{empty}");
    }

    #[test]
    fn checks_amd_dll() {
        assert!(matches!(check_amd_dll(b"MZ not it"), Err(Error::Integrity(_))));
        assert_eq!(github::git_blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391", "git's empty blob");
    }

    #[test]
    fn filters_override_files() {
        let b = |p: &str| TreeBlob { path: p.into(), sha: "0".repeat(40), size: 1 };
        let tree = vec![
            b("prebuilt/000fd316e19e1bfe.spv"),
            b("prebuilt/ORIGINALS.sha256"),
            b("prebuilt-lossy/0aa3911f7253b2f6.spv"),
            b("prebuilt-lossy/sub/x.spv"),
            b("prebuiltx/y.spv"),
            b("research/z.spv"),
        ];
        let paths: Vec<&str> = override_blobs(&tree).iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, ["prebuilt/000fd316e19e1bfe.spv", "prebuilt-lossy/0aa3911f7253b2f6.spv"]);
    }

    #[test]
    fn downloads_and_checks_overrides() {
        let a = b"spirv one".to_vec();
        let c = b"spirv two".to_vec();
        let (sa, sc) = (github::git_blob_sha(&a), github::git_blob_sha(&c));
        let sha = "a".repeat(40);
        let commit = format!(r#"{{"sha":"{sha}","commit":{{"committer":{{"date":"2026-10-13T10:00:00Z"}}}}}}"#);
        let tree = format!(
            r#"{{"tree":[{{"path":"prebuilt/aa.spv","type":"blob","sha":"{sa}","size":9}},{{"path":"prebuilt-lossy/bb.spv","type":"blob","sha":"{sc}","size":9}},{{"path":"README.md","type":"blob","sha":"{sa}","size":9}}]}}"#
        );
        let serve = |lossy: &[u8]| {
            let (base, _) = crate::testutil::serve_bytes(vec![
                ("/repos/zgauthier2000/fsr4-shader-performance-overrides/commits/main", 200, commit.clone().into_bytes()),
                ("/repos/zgauthier2000/fsr4-shader-performance-overrides/git/trees/", 200, tree.clone().into_bytes()),
                ("/prebuilt/aa.spv", 200, a.clone()),
                ("/prebuilt-lossy/bb.spv", 200, lossy.to_vec()),
                ("/repos/zgauthier2000/fsr4-shader-performance-overrides", 200, br#"{"default_branch":"main"}"#.to_vec()),
            ]);
            github::Client::with_base(&base).unwrap()
        };
        let root = tempfile::tempdir().unwrap();
        let info = download_overrides(&serve(&c), root.path(), &mut |_, _| {}).unwrap();
        assert_eq!((info.commit.as_str(), info.exact, info.lossy), (sha.as_str(), 1, 1));
        assert_eq!(std::fs::read(root.path().join("prebuilt/aa.spv")).unwrap(), a);
        assert_eq!(std::fs::read(root.path().join("prebuilt-lossy/bb.spv")).unwrap(), c);
        assert_eq!(overrides_info(root.path()).unwrap(), info);

        // A file that doesn't match its blob id fails the whole download and
        // leaves the earlier files as they were.
        let fresh = tempfile::tempdir().unwrap();
        assert!(matches!(download_overrides(&serve(b"tampered"), fresh.path(), &mut |_, _| {}), Err(Error::Integrity(_))));
        assert!(overrides_info(fresh.path()).is_none());
        assert!(!fresh.path().join("prebuilt").exists());
        assert!(download_overrides(&serve(b"tampered"), root.path(), &mut |_, _| {}).is_err());
        assert_eq!(std::fs::read(root.path().join("prebuilt-lossy/bb.spv")).unwrap(), c);
    }

    #[test]
    fn remembers_the_choice() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        assert!(save_choice(state.path(), root.path(), Some(Variant::Exact)).is_err(), "not downloaded yet");
        std::fs::create_dir_all(root.path().join("prebuilt")).unwrap();
        save_choice(state.path(), root.path(), Some(Variant::Exact)).unwrap();
        assert_eq!(load_choice(state.path()).unwrap(), Choice { variant: Variant::Exact, folder: root.path().join("prebuilt") });
        save_choice(state.path(), root.path(), None).unwrap();
        assert!(load_choice(state.path()).is_none());
        assert_eq!(override_value(Path::new("/home/a b/x")).as_deref(), Some("Z:/home/a b/x"));
        assert!(override_value(Path::new("/home/it's")).is_none());
    }
}
