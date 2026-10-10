// Commands for the patch-note check card in the Installed mods tab. The
// logic lives in cp2077mm-core's patch_notes module.

use cp2077mm_core::Result;
use cp2077mm_core::patch_notes::{self, Report};
use tauri::{AppHandle, Manager};

use super::{AppState, SOURCES, blocking, nexus_client};

/// Installed mods against the bundled patch list (offline).
#[tauri::command]
pub async fn patch_notes_report(app: AppHandle, game_id: i64) -> Result<Report> {
    blocking(move || patch_notes::report(&app.state::<AppState>().db.lock().unwrap(), game_id)).await
}

/// Read release dates from Nexus and GitHub, keep them, and report again.
#[tauri::command]
pub async fn patch_notes_read_dates(app: AppHandle, game_id: i64) -> Result<Report> {
    blocking(move || {
        let mods: Vec<_> = app.state::<AppState>().db.lock().unwrap().mods(game_id)?.into_iter().filter(|m| m.enabled()).collect();
        let nexus = nexus_client().ok();
        let fetched = patch_notes::fetch(&mods, nexus.as_ref(), &SOURCES);
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        patch_notes::save_fetched(&db, &mods, &fetched)?;
        let mut r = patch_notes::report(&db, game_id)?;
        r.errors = fetched.errors;
        Ok(r)
    })
    .await
}
