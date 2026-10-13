// Commands for the Radio stations section of Installed mods. The logic lives
// in cp2077mm-core's radioext module.

use std::path::Path;

use cp2077mm_core::Result;
use cp2077mm_core::install::InstallReport;
use cp2077mm_core::radioext::{self, Draft, SongInfo, Status};
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
