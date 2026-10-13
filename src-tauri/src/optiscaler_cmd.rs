// Commands for the OptiScaler page in the Installed mods tab: OptiScaler from
// GitHub releases, AMD's FSR 4.1.1 upscaler DLL, and the FSR 4.1.1 shader
// overrides for vkd3d-proton. The logic lives in cp2077mm-core's optiscaler
// module; the launch option itself is a Linux setup check (fsr4-overrides).

use std::path::PathBuf;

use cp2077mm_core::install::InstallReport;
use cp2077mm_core::optiscaler::{self, OverridesInfo, Status, Variant};
use cp2077mm_core::{Error, Result, paths};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::reshade_cmd::github;
use super::{AppState, blocking, with_installer, with_setup};

#[derive(Serialize, Clone)]
struct Progress {
    what: &'static str,
    done: u64,
    total: u64,
}

/// Release archives and AMD's DLL are kept here.
fn download_dir() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("optiscaler"))
}

/// The shader overrides, shared by every game install.
fn overrides_root() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("fsr4-shader-overrides"))
}

#[tauri::command]
pub async fn optiscaler_status(app: AppHandle, game_id: i64) -> Result<Status> {
    blocking(move || {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let game = db.game(game_id)?;
        optiscaler::status(&db, &game)
    })
    .await
}

#[derive(Serialize)]
pub struct Latest {
    tag: String,
    asset: String,
    url: String,
}

/// The newest stable OptiScaler release on GitHub.
#[tauri::command]
pub async fn optiscaler_latest() -> Result<Latest> {
    blocking(|| {
        let releases = github()?.releases(optiscaler::OWNER, optiscaler::REPO)?;
        let r = optiscaler::latest_stable(&releases).ok_or_else(|| Error::Other("OptiScaler: no stable release with an archive".into()))?;
        let a = optiscaler::pick_asset(r).map(|a| a.name.clone()).unwrap_or_default();
        Ok(Latest { tag: r.tag.clone(), asset: a, url: r.url.clone() })
    })
    .await
}

/// Download the newest stable OptiScaler (and, with `fsr`, AMD's FSR 4.1.1
/// upscaler DLL), check them and install or update OptiScaler.
#[tauri::command]
pub async fn optiscaler_install(app: AppHandle, game_id: i64, proxy: Option<String>, fsr: bool) -> Result<InstallReport> {
    blocking(move || {
        let gh = github()?;
        let releases = gh.releases(optiscaler::OWNER, optiscaler::REPO)?;
        let r = optiscaler::latest_stable(&releases).ok_or_else(|| Error::Other("OptiScaler: no stable release with an archive".into()))?;
        let asset = optiscaler::pick_asset(r).ok_or_else(|| Error::Other("OptiScaler: no archive in the release".into()))?;
        let dir = download_dir()?;
        let dl = gh.download(optiscaler::OWNER, optiscaler::REPO, asset, &dir, &mut |done, total| {
            let _ = app.emit("optiscaler-progress", Progress { what: "OptiScaler", done, total });
        })?;
        let amd = if fsr {
            let _ = app.emit("optiscaler-progress", Progress { what: "AMD FSR 4.1.1 DLL", done: 0, total: 0 });
            Some(optiscaler::download_amd_dll(&gh, &dir)?)
        } else {
            None
        };
        let result = {
            let state = app.state::<AppState>();
            let db = state.db.lock().unwrap();
            let game = db.game(game_id)?;
            with_installer(&db, |i| optiscaler::install(i, &game, &dl.path, &r.tag, proxy.as_deref(), amd.as_deref()))
        };
        // The installed copy is kept in staging; the archive isn't needed.
        let _ = std::fs::remove_file(&dl.path);
        result
    })
    .await
}

#[derive(Serialize)]
pub struct Overrides {
    /// What is downloaded, if anything.
    info: Option<OverridesInfo>,
    /// The variant this game's launch options should load.
    choice: Option<Variant>,
    /// The overrides are a Proton (vkd3d-proton) feature.
    available: bool,
}

#[tauri::command]
pub async fn fsr4_overrides(app: AppHandle, game_id: i64) -> Result<Overrides> {
    blocking(move || {
        let info = optiscaler::overrides_info(&overrides_root()?);
        let choice = with_setup(&app, game_id, |ctx| Ok(optiscaler::load_choice(&ctx.state_dir).map(|c| c.variant)))?;
        Ok(Overrides { info, choice, available: !cfg!(windows) })
    })
    .await
}

/// Download both variants at the repository's newest commit.
#[tauri::command]
pub async fn fsr4_overrides_download(app: AppHandle) -> Result<OverridesInfo> {
    blocking(move || {
        optiscaler::download_overrides(&github()?, &overrides_root()?, &mut |done, total| {
            let _ = app.emit("optiscaler-progress", Progress { what: "shader overrides", done, total });
        })
    })
    .await
}

/// Pick the variant this game's launch options load, or none.
#[tauri::command]
pub async fn fsr4_overrides_choose(app: AppHandle, game_id: i64, variant: Option<Variant>) -> Result<()> {
    blocking(move || {
        let root = overrides_root()?;
        with_setup(&app, game_id, |ctx| optiscaler::save_choice(&ctx.state_dir, &root, variant))
    })
    .await
}
