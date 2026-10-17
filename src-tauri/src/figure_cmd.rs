// Commands for Installed mods › Canvas › Figure: which body or clothing slot
// a mod's files point at. The logic lives in cp2077mm-core's figure module.

use cp2077mm_core::figure::{self, Hints};
use cp2077mm_core::wolvenkit::{self, Cli};
use cp2077mm_core::{Result, paths};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use super::{AppState, blocking};

/// Slot hints from tweaks, ArchiveXL files and cached WolvenKit listings.
#[tauri::command]
pub async fn figure_hints(app: AppHandle, game_id: i64) -> Result<Hints> {
    blocking(move || {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        figure::hints_for_game(&db, &paths::staging_dir()?, &figure::cache_dir()?, game_id)
    })
    .await
}

#[derive(Serialize)]
pub struct ReadArchives {
    tool: String,
    read: usize,
    failed: Vec<String>,
}

/// List the `.archive` files of `mod_ids` with the WolvenKit CLI and cache
/// the depot paths by file hash. Archives already cached are skipped.
#[tauri::command]
pub async fn figure_read_archives(app: AppHandle, mod_ids: Vec<i64>) -> Result<ReadArchives> {
    blocking(move || {
        let cli = Cli::find(&wolvenkit::root()?)?;
        let cache = figure::cache_dir()?;
        let staging = paths::staging_dir()?;
        // Collect under the lock, run WolvenKit without it.
        let todo: Vec<_> = {
            let state = app.state::<AppState>();
            let db = state.db.lock().unwrap();
            let mut v = Vec::new();
            for id in mod_ids {
                v.extend(figure::mod_archives(&db, &staging, id)?);
            }
            v
        };
        let (mut read, mut failed) = (0, Vec::new());
        for (path, sha) in todo {
            if figure::cached_listing(&cache, &sha).is_some() {
                continue;
            }
            match cli.list_archive(&path) {
                Ok(list) => {
                    figure::store_listing(&cache, &sha, &list)?;
                    read += 1;
                }
                Err(e) => failed.push(format!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy())),
            }
        }
        Ok(ReadArchives { tool: format!("WolvenKit CLI {}", cli.version), read, failed })
    })
    .await
}
