// Commands for the WolvenKit section of the Netrunner tab: WolvenKit from
// its GitHub releases, the .NET runtime its CLI needs, and reading archives
// with the CLI. The logic lives in cp2077mm-core's wolvenkit module.

use std::path::{Path, PathBuf};

use cp2077mm_core::wolvenkit::{self, Cli, Installed, Part, Status};
use cp2077mm_core::{Error, Result, desktop};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::blocking;
use super::reshade_cmd::github;

#[derive(Serialize, Clone)]
struct Progress {
    what: &'static str,
    done: u64,
    total: u64,
}

#[tauri::command]
pub async fn wolvenkit_status() -> Result<Status> {
    blocking(|| Ok(wolvenkit::status(&wolvenkit::root()?))).await
}

#[derive(Serialize)]
pub struct Latest {
    tag: String,
    url: String,
    published: Option<i64>,
    /// Asset names by part, where the release has one for this system.
    cli: Option<String>,
    editor: Option<String>,
}

/// The newest stable WolvenKit release on GitHub.
#[tauri::command]
pub async fn wolvenkit_latest() -> Result<Latest> {
    blocking(|| {
        let releases = github()?.releases(wolvenkit::OWNER, wolvenkit::REPO)?;
        let r = wolvenkit::latest_stable(&releases, Part::Cli).ok_or_else(|| Error::Other("WolvenKit: no stable release with a CLI build".into()))?;
        let name = |p: Part| p.available().then(|| wolvenkit::pick_asset(r, p).map(|a| a.name.clone())).flatten();
        Ok(Latest { tag: r.tag.clone(), url: r.url.clone(), published: r.published, cli: name(Part::Cli), editor: name(Part::Editor) })
    })
    .await
}

/// Download the newest stable release of `part`, check its SHA-256 against
/// GitHub's digest and unpack it into the app's data folder.
#[tauri::command]
pub async fn wolvenkit_install(app: AppHandle, part: Part) -> Result<Installed> {
    if !part.available() {
        return Err(Error::Other(format!("{} is a Windows program", part.label())));
    }
    blocking(move || {
        let gh = github()?;
        let releases = gh.releases(wolvenkit::OWNER, wolvenkit::REPO)?;
        let r = wolvenkit::latest_stable(&releases, part).ok_or_else(|| Error::Other(format!("WolvenKit: no stable release with the {}", part.label())))?;
        let asset = wolvenkit::pick_asset(r, part).ok_or_else(|| Error::Other("WolvenKit: asset not found".into()))?;
        let root = wolvenkit::root()?;
        let dl = gh.download(wolvenkit::OWNER, wolvenkit::REPO, asset, &root.join("downloads"), &mut |done, total| {
            let _ = app.emit("wolvenkit-progress", Progress { what: part.label(), done, total });
        })?;
        let result = wolvenkit::install(&root, part, &dl.path, &r.tag, &asset.name, &dl.sha256, dl.verified);
        let _ = std::fs::remove_file(&dl.path);
        result
    })
    .await
}

#[tauri::command]
pub async fn wolvenkit_remove(part: Part) -> Result<()> {
    blocking(move || wolvenkit::remove(&wolvenkit::root()?, part)).await
}

#[tauri::command]
pub async fn wolvenkit_open_editor() -> Result<()> {
    blocking(|| wolvenkit::open_editor(&wolvenkit::root()?)).await
}

#[tauri::command]
pub async fn wolvenkit_open_folder() -> Result<()> {
    blocking(|| {
        let root = wolvenkit::root()?;
        std::fs::create_dir_all(&root)?;
        desktop::open_folder(&root)
    })
    .await
}

#[derive(Serialize)]
pub struct Listing {
    tool: String,
    files: Vec<String>,
    /// Lines left out past the limit.
    more: usize,
}

const MAX_LISTED: usize = 5000;

/// The files in an `.archive` the user picked, as WolvenKit lists them.
#[tauri::command]
pub async fn wolvenkit_list_archive(path: String) -> Result<Listing> {
    blocking(move || {
        let path = PathBuf::from(path);
        if !path.is_file() || !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("archive")) {
            return Err(Error::Other(format!("{} is not an .archive file", path.display())));
        }
        let cli = Cli::find(&wolvenkit::root()?)?;
        let mut files = cli.list_archive(Path::new(&path))?;
        let more = files.len().saturating_sub(MAX_LISTED);
        files.truncate(MAX_LISTED);
        Ok(Listing { tool: format!("WolvenKit CLI {}", cli.version), files, more })
    })
    .await
}
