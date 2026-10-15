//! Mods in the game folder that CPMX2077 didn't install (copied in by hand,
//! or put there by another manager), grouped into units the way they are
//! installed: one `.archive` with its ArchiveXL `.xl` files, one REDmod
//! folder, one CET mod folder, one RED4ext plugin, one entry in
//! `r6/scripts` or `r6/tweaks`.
//!
//! They are scanned in place for the compatibility report and the File map's
//! wins/loses, and can be adopted: their files are copied into a stored copy
//! and recorded like an installed mod, so Enabled, Verify and Uninstall work
//! on them. Adopting changes nothing in the game folder.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::activity::{self, Kind as ActivityKind};
use crate::analysis::{Touch, scan_mod};
use crate::db::{Db, GameRow, ModFile, NewMod};
use crate::install::{Installer, resolve_ci};
use crate::{Error, Result, hash};

/// Source of mods adopted from the game folder.
pub const SOURCE: &str = "adopted";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Archive,
    Redmod,
    Cet,
    Red4ext,
    Redscript,
    Tweak,
}

impl UnitKind {
    pub fn label(self) -> &'static str {
        match self {
            UnitKind::Archive => "Archive",
            UnitKind::Redmod => "REDmod",
            UnitKind::Cet => "CET",
            UnitKind::Red4ext => "RED4ext plugin",
            UnitKind::Redscript => "Redscript",
            UnitKind::Tweak => "TweakXL",
        }
    }
}

/// One untracked mod.
#[derive(Debug, Clone, Serialize)]
pub struct Unit {
    /// Game-relative path of the archive, folder or file that makes the
    /// unit; unique per game.
    pub key: String,
    pub name: String,
    pub kind: UnitKind,
    /// Game-relative paths of its files.
    pub files: Vec<String>,
    pub size: u64,
    /// Newest file modification time (unix seconds).
    pub modified: Option<i64>,
}

/// Where untracked units are looked for, and how.
const CET_MODS: &str = "bin/x64/plugins/cyber_engine_tweaks/mods";
const MOD_DIR: &str = "archive/pc/mod";

/// RED4ext plugins that are core frameworks; the Game panel lists them.
const FRAMEWORK_PLUGINS: &[&str] = &["archivexl", "tweakxl", "codeware"];

/// Files never counted as a mod's: logs, and files load-order tools keep
/// next to the archives.
fn ignored(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.ends_with(".log") || l.ends_with(".disabled")
}

fn list_dir(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| !t.is_symlink()))
                .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Every regular file under `dir` (not following symlinks), as paths
/// relative to `rel` (the game-relative path of `dir`).
fn files_under(dir: &Path, rel: &str) -> Vec<String> {
    if dir.is_file() {
        return vec![rel.to_string()];
    }
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(dir).follow_links(false).into_iter().flatten() {
        if !e.file_type().is_file() {
            continue;
        }
        let Ok(sub) = e.path().strip_prefix(dir) else { continue };
        let Some(sub) = sub.to_str() else { continue };
        out.push(format!("{rel}/{}", sub.replace('\\', "/")));
    }
    out.sort();
    out
}

fn stats(game_dir: &Path, files: &[String]) -> (u64, Option<i64>) {
    let mut size = 0;
    let mut newest = None;
    for f in files {
        let Ok(m) = std::fs::metadata(resolve_ci(game_dir, f)) else { continue };
        size += m.len();
        let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
        newest = newest.max(t);
    }
    (size, newest)
}

/// Lowercased game-relative paths CPMX2077 tracks in this game.
fn tracked_paths(db: &Db, game: &GameRow) -> Result<HashSet<String>> {
    let mut st = db.conn.prepare(
        "SELECT lower(f.rel_path) FROM mod_files f JOIN mods m ON m.id = f.mod_id WHERE m.game_id = ?1",
    )?;
    let rows = st.query_map([game.id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The untracked mods in `game`, sorted by kind and name.
pub fn scan(db: &Db, game: &GameRow) -> Result<Vec<Unit>> {
    Ok(scan_dir(Path::new(&game.path), &tracked_paths(db, game)?))
}

/// `scan` against a set of tracked (lowercased) paths. A folder unit that
/// holds any tracked file belongs to that mod (its other files are ones the
/// mod wrote at runtime), so it is left out.
pub fn scan_dir(game_dir: &Path, tracked: &HashSet<String>) -> Vec<Unit> {
    let is_tracked = |p: &String| tracked.contains(&p.to_lowercase());
    let mut units = Vec::new();
    let mut push = |kind: UnitKind, key: String, name: String, files: Vec<String>| {
        let files: Vec<String> = files.into_iter().filter(|f| !ignored(f)).collect();
        if files.is_empty() || files.iter().any(is_tracked) {
            return;
        }
        let (size, modified) = stats(game_dir, &files);
        units.push(Unit { key, name, kind, files, size, modified });
    };

    // archive/pc/mod: each .archive with the .xl files named after it.
    let entries = list_dir(&resolve_ci(game_dir, MOD_DIR));
    let lower = |s: &str| s.to_ascii_lowercase();
    let mut xls: BTreeMap<String, String> = entries
        .iter()
        .filter(|(n, p)| p.is_file() && lower(n).ends_with(".xl"))
        .map(|(n, _)| (lower(n), n.clone()))
        .collect();
    for (name, path) in &entries {
        let l = lower(name);
        if !path.is_file() || !l.ends_with(".archive") {
            continue;
        }
        let stem = &l[..l.len() - ".archive".len()];
        let archive = format!("{MOD_DIR}/{name}");
        // An archive CPMX2077 tracks keeps its .xl files out of the pairing:
        // untracked ones are then listed on their own below.
        if is_tracked(&archive) {
            continue;
        }
        let mut files = vec![archive.clone()];
        for xl in [format!("{stem}.xl"), format!("{l}.xl")] {
            if let Some(real) = xls.remove(&xl) {
                let rel = format!("{MOD_DIR}/{real}");
                if !is_tracked(&rel) {
                    files.push(rel);
                }
            }
        }
        push(UnitKind::Archive, archive, name[..name.len() - ".archive".len()].to_string(), files);
    }
    for (_, real) in xls {
        let stem = real[..real.len() - ".xl".len()].to_string();
        push(UnitKind::Archive, format!("{MOD_DIR}/{real}"), stem, vec![format!("{MOD_DIR}/{real}")]);
    }

    // One unit per folder (or loose file) under these roots.
    let roots: [(&str, UnitKind); 5] = [
        ("mods", UnitKind::Redmod),
        (CET_MODS, UnitKind::Cet),
        ("red4ext/plugins", UnitKind::Red4ext),
        ("r6/scripts", UnitKind::Redscript),
        ("r6/tweaks", UnitKind::Tweak),
    ];
    for (root, kind) in roots {
        for (name, path) in list_dir(&resolve_ci(game_dir, root)) {
            if kind == UnitKind::Red4ext && FRAMEWORK_PLUGINS.contains(&lower(&name).as_str()) {
                continue;
            }
            // REDmod and CET mods are folders; loose files there aren't mods.
            if path.is_file() && matches!(kind, UnitKind::Redmod | UnitKind::Cet) {
                continue;
            }
            let key = format!("{root}/{name}");
            let files = files_under(&path, &key);
            let display = if path.is_file() { Path::new(&name).file_stem().and_then(|s| s.to_str()).unwrap_or(&name).to_string() } else { name.clone() };
            push(kind, key, display, files);
        }
    }
    units
}

// ---------------------------------------------------------------------------
// Analysis

type CacheKey = (PathBuf, u64, Option<std::time::SystemTime>);

/// Scans of archives in the game folder, so a report doesn't re-read every
/// archive's index each time. Keyed by path, size and modification time.
static SCAN_CACHE: Mutex<Option<HashMap<CacheKey, Vec<Touch>>>> = Mutex::new(None);

/// What `unit` touches, scanned from the game folder.
pub fn touches(game_dir: &Path, unit: &Unit) -> Vec<Touch> {
    let mut out = Vec::new();
    for f in &unit.files {
        let path = resolve_ci(game_dir, f);
        let meta = std::fs::metadata(&path).ok();
        let key: CacheKey = (path.clone(), meta.as_ref().map_or(0, |m| m.len()), meta.and_then(|m| m.modified().ok()));
        if let Some(hit) = SCAN_CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
            out.extend(hit.iter().cloned());
            continue;
        }
        let t = scan_mod(&[(f.clone(), path)]);
        SCAN_CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, t.clone());
        out.extend(t);
    }
    out.sort();
    out.dedup();
    out
}

/// Name shown for an untracked unit in reports.
pub fn report_name(u: &Unit) -> String {
    format!("{} (untracked)", u.name)
}

// ---------------------------------------------------------------------------
// Adopting

/// Start tracking the untracked unit `key` as a mod: copy its files into a
/// stored copy and record their hashes. The game folder is not changed.
pub fn adopt(inst: &Installer, game: &GameRow, key: &str) -> Result<i64> {
    let game_dir = PathBuf::from(&game.path);
    let unit = scan(inst.db, game)?
        .into_iter()
        .find(|u| u.key == key)
        .ok_or_else(|| Error::Other(format!("{key} is not an untracked mod in this game")))?;
    let meta = NewMod {
        name: unit.name.clone(),
        source: SOURCE.into(),
        archive_name: unit.key.clone(),
        category: None,
        ..Default::default()
    };
    let mod_id = inst.db.insert_mod(game, &meta)?;
    let stage_dir = inst.staging_root.join(mod_id.to_string());
    let r = (|| -> Result<()> {
        for f in &unit.files {
            let src = resolve_ci(&game_dir, f);
            if !src.starts_with(&game_dir) || src.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(Error::UnsafeArchive(format!("{f} is outside the game folder or a symlink")));
            }
            let dst = stage_dir.join(f);
            std::fs::create_dir_all(dst.parent().unwrap_or(&stage_dir))?;
            std::fs::copy(&src, &dst)?;
            let sha = hash::sha256_file(&dst)?;
            let size = std::fs::metadata(&dst)?.len() as i64;
            inst.db.add_mod_file(&ModFile { mod_id, rel_path: f.clone(), staged_path: f.clone(), sha256: sha, size })?;
        }
        // Date it by its files, so the Installed column shows when it went in.
        if let Some(t) = unit.modified {
            inst.db.conn.execute("UPDATE mods SET installed_at = datetime(?2, 'unixepoch') WHERE id = ?1", rusqlite::params![mod_id, t])?;
        }
        Ok(())
    })();
    if let Err(e) = r {
        let _ = inst.db.delete_mod(mod_id);
        let _ = std::fs::remove_dir_all(&stage_dir);
        return Err(e);
    }
    activity::record_path(ActivityKind::Copy, format!("Adopted {} ({} files); stored copy", unit.name, unit.files.len()), &stage_dir);
    if let Err(e) = crate::analysis::index_mod(inst.db, &inst.staging_root, mod_id) {
        log::warn!("indexing adopted mod {mod_id} failed: {e}");
    }
    Ok(mod_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Limits;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn game_tree() -> tempfile::TempDir {
        let g = tempfile::tempdir().unwrap();
        let r = g.path();
        write(r, "archive/pc/mod/Alpha.archive", "x");
        write(r, "archive/pc/mod/Alpha.archive.xl", "x");
        write(r, "archive/pc/mod/beta.archive", "x");
        write(r, "archive/pc/mod/beta.xl", "x");
        write(r, "archive/pc/mod/lonely.xl", "x");
        write(r, "archive/pc/mod/modlist.txt", "x");
        write(r, "archive/pc/mod/old.archive.disabled", "x");
        write(r, "bin/x64/plugins/cyber_engine_tweaks/mods/Menu/init.lua", "x");
        write(r, "bin/x64/plugins/cyber_engine_tweaks/mods/Menu/data/a.json", "x");
        write(r, "bin/x64/plugins/cyber_engine_tweaks/mods/Menu/Menu.log", "x");
        write(r, "red4ext/plugins/ArchiveXL/ArchiveXL.dll", "x");
        write(r, "red4ext/plugins/Hud/Hud.dll", "x");
        write(r, "r6/scripts/loose.reds", "x");
        write(r, "r6/scripts/Folder/a.reds", "x");
        write(r, "r6/tweaks/t.yaml", "x");
        write(r, "mods/Red/info.json", "{}");
        g
    }

    #[test]
    fn groups_units_and_skips_frameworks_logs_and_markers() {
        let g = game_tree();
        let units = scan_dir(g.path(), &HashSet::new());
        let got: Vec<(&str, Vec<&str>)> = units.iter().map(|u| (u.key.as_str(), u.files.iter().map(String::as_str).collect())).collect();
        assert_eq!(
            got,
            vec![
                ("archive/pc/mod/Alpha.archive", vec!["archive/pc/mod/Alpha.archive", "archive/pc/mod/Alpha.archive.xl"]),
                ("archive/pc/mod/beta.archive", vec!["archive/pc/mod/beta.archive", "archive/pc/mod/beta.xl"]),
                ("archive/pc/mod/lonely.xl", vec!["archive/pc/mod/lonely.xl"]),
                ("mods/Red", vec!["mods/Red/info.json"]),
                ("bin/x64/plugins/cyber_engine_tweaks/mods/Menu", vec![
                    "bin/x64/plugins/cyber_engine_tweaks/mods/Menu/data/a.json",
                    "bin/x64/plugins/cyber_engine_tweaks/mods/Menu/init.lua",
                ]),
                ("red4ext/plugins/Hud", vec!["red4ext/plugins/Hud/Hud.dll"]),
                ("r6/scripts/Folder", vec!["r6/scripts/Folder/a.reds"]),
                ("r6/scripts/loose.reds", vec!["r6/scripts/loose.reds"]),
                ("r6/tweaks/t.yaml", vec!["r6/tweaks/t.yaml"]),
            ]
        );
        assert_eq!(units[0].name, "Alpha");
        assert_eq!(units[7].name, "loose");
        assert_eq!(units[0].size, 2);
        assert!(units[0].modified.is_some());
    }

    #[test]
    fn folder_with_a_tracked_file_belongs_to_that_mod() {
        let g = game_tree();
        let tracked: HashSet<String> =
            ["bin/x64/plugins/cyber_engine_tweaks/mods/menu/init.lua", "archive/pc/mod/beta.archive"].iter().map(|s| s.to_string()).collect();
        let keys: Vec<String> = scan_dir(g.path(), &tracked).into_iter().map(|u| u.key).collect();
        assert!(!keys.iter().any(|k| k.ends_with("/Menu")), "{keys:?}");
        assert!(!keys.contains(&"archive/pc/mod/beta.archive".to_string()));
        // beta's .xl isn't tracked, so it is listed on its own.
        assert!(keys.contains(&"archive/pc/mod/beta.xl".to_string()), "{keys:?}");
    }

    #[test]
    fn adopt_tracks_files_without_touching_the_game() {
        let g = game_tree();
        let data = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let gi = crate::game::GameInstall {
            path: g.path().to_path_buf(),
            store: crate::game::Store::Manual,
            proton_prefix: None,
            build_id: None,
            exe_file_version: None,
            exe_product_version: None,
            frameworks: vec![],
            launch_options: None,
            warnings: vec![],
        };
        let game = db.game(db.upsert_game(&gi).unwrap()).unwrap();
        let inst = Installer { db: &db, staging_root: data.path().join("staging"), backups_root: data.path().join("backups"), limits: Limits::default() };

        let id = adopt(&inst, &game, "bin/x64/plugins/cyber_engine_tweaks/mods/Menu").unwrap();
        let m = db.get_mod(id).unwrap();
        assert_eq!((m.name.as_str(), m.source.as_str(), m.file_count), ("Menu", SOURCE, 2));
        assert!(g.path().join("bin/x64/plugins/cyber_engine_tweaks/mods/Menu/init.lua").is_file());
        assert!(data.path().join(format!("staging/{id}/bin/x64/plugins/cyber_engine_tweaks/mods/Menu/init.lua")).is_file());
        assert!(!scan(&db, &game).unwrap().iter().any(|u| u.key.ends_with("/Menu")));
        assert_eq!(inst.verify(id).unwrap().ok, 2);

        // Disable and enable work from the stored copy; uninstall removes it.
        inst.disable(id).unwrap();
        assert!(!g.path().join("bin/x64/plugins/cyber_engine_tweaks/mods/Menu/init.lua").exists());
        inst.enable(id, false).unwrap();
        assert!(g.path().join("bin/x64/plugins/cyber_engine_tweaks/mods/Menu/init.lua").is_file());
        inst.uninstall(id).unwrap();
        assert!(!g.path().join("bin/x64/plugins/cyber_engine_tweaks/mods/Menu/init.lua").exists());
        assert!(!data.path().join(format!("staging/{id}")).exists());

        assert!(adopt(&inst, &game, "../outside").is_err());
    }

    #[test]
    fn untracked_archives_show_in_the_compatibility_report() {
        use crate::analysis::{depot_hash, tests::rdar_bytes};
        let g = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let shared = depot_hash("base\\shared.mesh");
        let dir = g.path().join("archive/pc/mod");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("aaa.archive"), rdar_bytes(&[shared])).unwrap();
        std::fs::write(dir.join("bbb.archive"), rdar_bytes(&[shared, depot_hash("base\\other.mesh")])).unwrap();
        let db = Db::open_in_memory().unwrap();
        let gi = crate::game::GameInstall {
            path: g.path().to_path_buf(),
            store: crate::game::Store::Manual,
            proton_prefix: None,
            build_id: None,
            exe_file_version: None,
            exe_product_version: None,
            frameworks: vec![],
            launch_options: None,
            warnings: vec![],
        };
        let game = db.game(db.upsert_game(&gi).unwrap()).unwrap();
        let r = crate::analysis::report_for_game(&db, &data.path().join("staging"), &game).unwrap();
        let names: Vec<&str> = r.mods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"aaa (untracked)"));
        assert!(r.mods.iter().all(|m| m.mod_id < 0));
        let f = r.findings.iter().find(|f| f.kind == crate::analysis::Kind::Resource).expect("shared resource finding");
        assert_eq!(f.mod_ids.len(), 2);

        // The File map counts them too once a mod is adopted.
        let inst = Installer { db: &db, staging_root: data.path().join("staging"), backups_root: data.path().join("backups"), limits: Limits::default() };
        adopt(&inst, &game, "archive/pc/mod/aaa.archive").unwrap();
        let tree = crate::file_tree::build(&db, &game, crate::file_tree::Group::Location).unwrap();
        let mut files = vec![&tree.root];
        let mut aaa = None;
        while let Some(n) = files.pop() {
            files.extend(n.folders.iter());
            if let Some(f) = n.files.iter().find(|f| f.name == "aaa.archive") {
                aaa = Some((f.wins, f.loses));
            }
        }
        assert_eq!(aaa, Some((1, 0)));
    }
}
