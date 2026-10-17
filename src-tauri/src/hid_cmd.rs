// Commands for the HID input page in the Installed mods tab: the game
// controllers on this computer, the CPMX2077 HID Input plugin (a RED4ext
// plugin carried in this binary) and its bindings file. The logic lives in
// cp2077mm-core's hid_input module.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use cp2077mm_core::Result;
use cp2077mm_core::hid_input::{self, DeviceView, LiveView, PluginState, Scanner};
use cp2077mm_core::install::InstallReport;
use cp2077mm_hidmap::Bindings;
use tauri::{AppHandle, Manager};

use super::{AppState, blocking, with_installer};

/// The plugin DLL, built for Windows from crates/hid-plugin (empty when
/// this build was made without it; see build.rs).
static PLUGIN_DLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cpmx_hid_input.dll"));

/// Devices stay open between polls while the page is shown.
static SCANNER: LazyLock<Mutex<Scanner>> = LazyLock::new(Default::default);

fn game_dir(app: &AppHandle, game_id: i64) -> Result<PathBuf> {
    Ok(PathBuf::from(app.state::<AppState>().db.lock().unwrap().game(game_id)?.path))
}

#[tauri::command]
pub async fn hid_state(app: AppHandle, game_id: i64) -> Result<PluginState> {
    blocking(move || {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let game = db.game(game_id)?;
        hid_input::state(&db, &game, PLUGIN_DLL)
    })
    .await
}

/// List the connected game controllers again.
#[tauri::command]
pub async fn hid_devices() -> Result<Vec<DeviceView>> {
    blocking(|| Ok(SCANNER.lock().unwrap().devices())).await
}

#[tauri::command]
pub async fn hid_poll() -> Result<Vec<LiveView>> {
    blocking(|| Ok(SCANNER.lock().unwrap().poll())).await
}

#[tauri::command]
pub async fn hid_bindings(app: AppHandle, game_id: i64) -> Result<Bindings> {
    blocking(move || hid_input::load_bindings(&game_dir(&app, game_id)?)).await
}

#[tauri::command]
pub async fn hid_save_bindings(app: AppHandle, game_id: i64, bindings: Bindings) -> Result<()> {
    blocking(move || hid_input::save_bindings(&game_dir(&app, game_id)?, &bindings)).await
}

/// Install or update the plugin carried by this build.
#[tauri::command]
pub async fn hid_install(app: AppHandle, game_id: i64) -> Result<InstallReport> {
    blocking(move || {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let game = db.game(game_id)?;
        with_installer(&db, |i| hid_input::install(i, &game, PLUGIN_DLL))
    })
    .await
}
