// Commands for the RadioEXT section of Installed mods. The logic lives
// in cp2077mm-core's radioext module.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cp2077mm_core::install::InstallReport;
use cp2077mm_core::radioext::{self, Draft, SongInfo, Status};
use cp2077mm_core::wolvenkit::{self, ArchiveCheck};
use cp2077mm_core::{Result, inkicon};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use super::{AppState, blocking, with_installer};

/// radioExt in this game and every station in its `radios` folder.
#[tauri::command]
pub async fn radio_status(app: AppHandle, game_id: i64) -> Result<Status> {
    blocking(move || {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let game = db.game(game_id)?;
        with_installer(&db, |i| radioext::status(i, &game))
    })
    .await
}

/// Check picked song files (content, tags) and propose their names.
#[tauri::command]
pub async fn radio_song_info(paths: Vec<String>) -> Result<Vec<SongInfo>> {
    blocking(move || Ok(paths.iter().map(|p| radioext::song_info(Path::new(p))).collect())).await
}

/// Create a station, or save changes to one, as a tracked mod.
#[tauri::command]
pub async fn radio_save(app: AppHandle, game_id: i64, draft: Draft) -> Result<InstallReport> {
    blocking(move || {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let game = db.game(game_id)?;
        with_installer(&db, |i| radioext::save(i, &game, &draft))
    })
    .await
}

#[derive(Serialize)]
pub struct IconPreview {
    /// The icon as it goes into the texture, as a PNG data URL.
    data_url: String,
    /// Size of the picked image.
    width: u32,
    height: u32,
}

/// Check a picked PNG and show it as the station icon will look.
#[tauri::command]
pub async fn radio_icon_preview(path: String) -> Result<IconPreview> {
    blocking(move || {
        let img = inkicon::load_png(Path::new(&path))?;
        let square = inkicon::fit_square(&img, inkicon::ICON_SIZE);
        let png = inkicon::encode_png(&square)?;
        Ok(IconPreview { data_url: format!("data:image/png;base64,{}", STANDARD.encode(png)), width: img.width, height: img.height })
    })
    .await
}

/// Read a station's icon archive back with the WolvenKit CLI.
#[tauri::command]
pub async fn radio_icon_check(app: AppHandle, game_id: i64, folder: String) -> Result<ArchiveCheck> {
    blocking(move || {
        let game = app.state::<AppState>().db.lock().unwrap().game(game_id)?;
        let (archive, expected) = radioext::icon_files(&game, &folder)?;
        let cli = wolvenkit::Cli::find(&wolvenkit::root()?)?;
        wolvenkit::check_archive(&cli, &archive, &expected)
    })
    .await
}
