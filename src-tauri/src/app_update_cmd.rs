// Commands for the CPMX2077 update notice and its Settings card. The logic
// lives in cp2077mm-core's app_update module.

use cp2077mm_core::app_update::{self, DISMISSED_SETTING, STARTUP_SETTING, Status};
use cp2077mm_core::{APP_VERSION, Result};
use serde::Serialize;
use tauri::State;

use super::{AppState, blocking};
use crate::reshade_cmd::github;

#[derive(Serialize)]
pub struct Prefs {
    current: &'static str,
    on_startup: bool,
    /// Tag whose startup notice was dismissed.
    dismissed: Option<String>,
}

#[tauri::command]
pub fn app_update_prefs(state: State<'_, AppState>) -> Result<Prefs> {
    let db = state.db.lock().unwrap();
    Ok(Prefs {
        current: APP_VERSION,
        on_startup: db.get_setting(STARTUP_SETTING)?.as_deref() != Some("0"),
        dismissed: db.get_setting(DISMISSED_SETTING)?,
    })
}

#[tauri::command]
pub fn set_app_update_on_startup(state: State<'_, AppState>, on: bool) -> Result<()> {
    state.db.lock().unwrap().set_setting(STARTUP_SETTING, if on { "1" } else { "0" })
}

/// Hide the startup notice for this release (a newer one shows again).
#[tauri::command]
pub fn app_update_dismiss(state: State<'_, AppState>, tag: String) -> Result<()> {
    state.db.lock().unwrap().set_setting(DISMISSED_SETTING, &tag)
}

/// Compare this build with the newest CPMX2077 release on GitHub. A fresh
/// client each time, so "Check now" never answers from a cache.
#[tauri::command]
pub async fn app_update_check() -> Result<Status> {
    blocking(|| app_update::check(&github()?)).await
}
