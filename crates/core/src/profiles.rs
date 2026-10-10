//! Named mod profiles, filed under the game version they were made for. A
//! profile is a list of mods; applying it enables those mods and disables
//! every other installed mod, in the same game folder.
//!
//! Mods are matched by [`mod_key`] so a profile still finds a mod after it
//! was updated. Where one mod has several files installed (a Nexus main file
//! plus an optional file), the archive name picks out the ones in the
//! profile.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::db::{Db, ModRow};
use crate::install::Installer;
use crate::modpacks::mod_key;
use crate::{Error, Result};

pub const MAX_NAME: usize = 60;

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub id: i64,
    /// The game version it is filed under; empty when unknown.
    pub game_version: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub mod_count: usize,
    /// The enabled mods match the profile right now.
    pub active: bool,
    /// What applying it would change.
    pub to_enable: usize,
    pub to_disable: usize,
    /// Mods in the profile that are no longer installed.
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Entry {
    key: String,
    archive: String,
    name: String,
}

/// Installed mods to switch to match a profile.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    pub enable: Vec<i64>,
    pub disable: Vec<i64>,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ApplyReport {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    /// Files left in the game by disabled mods because they changed after
    /// install (settings).
    pub kept_files: usize,
    /// Mods that could not be switched, with the reason.
    pub failed: Vec<(String, String)>,
    pub missing: Vec<String>,
}

fn clean_name(name: &str) -> Result<String> {
    let n = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() {
        return Err(Error::Other("profile name is empty".into()));
    }
    if n.chars().count() > MAX_NAME {
        return Err(Error::Other(format!("profile name is longer than {MAX_NAME} characters")));
    }
    Ok(n)
}

fn entries(db: &Db, id: i64) -> Result<Vec<Entry>> {
    let mut st = db.conn.prepare("SELECT mod_key, archive_name, name FROM profile_mods WHERE profile_id = ?1")?;
    let rows = st.query_map([id], |r| Ok(Entry { key: r.get(0)?, archive: r.get(1)?, name: r.get(2)? }))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn plan_for(list: &[Entry], mods: &[ModRow]) -> Plan {
    let mut by_key: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for e in list {
        by_key.entry(&e.key).or_default().insert(&e.archive);
    }
    let keyed: Vec<(String, &ModRow)> = mods.iter().map(|m| (mod_key(m), m)).collect();
    let mut plan = Plan::default();
    for (key, m) in &keyed {
        let wanted = match by_key.get(key.as_str()) {
            None => false,
            Some(archives) => {
                // Exact files when any of them is still installed; otherwise
                // the mod was updated since, so every installed file counts.
                let exact = keyed.iter().any(|(k, o)| k == key && archives.contains(o.archive_name.as_str()));
                !exact || archives.contains(m.archive_name.as_str())
            }
        };
        if wanted && !m.enabled() {
            plan.enable.push(m.id);
        } else if !wanted && m.enabled() {
            plan.disable.push(m.id);
        }
    }
    let installed: BTreeSet<&str> = keyed.iter().map(|(k, _)| k.as_str()).collect();
    let mut missing: BTreeSet<&str> = BTreeSet::new();
    for e in list {
        if !installed.contains(e.key.as_str()) {
            missing.insert(&e.name);
        }
    }
    plan.missing = missing.into_iter().map(str::to_string).collect();
    plan
}

/// Every profile of a game, by game version then name.
pub fn list(db: &Db, game_id: i64) -> Result<Vec<Profile>> {
    let mods = db.mods(game_id)?;
    let mut st = db.conn.prepare(
        "SELECT id, game_version, name, created_at, updated_at FROM profiles
         WHERE game_id = ?1 ORDER BY game_version, name COLLATE NOCASE",
    )?;
    let rows: Vec<(i64, String, String, String, String)> = st
        .query_map([game_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, game_version, name, created_at, updated_at) in rows {
        let list = entries(db, id)?;
        let plan = plan_for(&list, &mods);
        out.push(Profile {
            id,
            game_version,
            name,
            created_at,
            updated_at,
            mod_count: list.iter().map(|e| &e.key).collect::<BTreeSet<_>>().len(),
            active: plan.enable.is_empty() && plan.disable.is_empty(),
            to_enable: plan.enable.len(),
            to_disable: plan.disable.len(),
            missing: plan.missing,
        });
    }
    Ok(out)
}

fn game_of(db: &Db, id: i64) -> Result<i64> {
    db.conn
        .query_row("SELECT game_id FROM profiles WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| Error::Other("profile not found".into()))
}

fn store_enabled(db: &Db, id: i64, game_id: i64) -> Result<()> {
    let tx = db.conn.unchecked_transaction()?;
    tx.execute("DELETE FROM profile_mods WHERE profile_id = ?1", [id])?;
    for m in db.mods(game_id)?.iter().filter(|m| m.enabled()) {
        tx.execute(
            "INSERT OR IGNORE INTO profile_mods (profile_id, mod_key, archive_name, name) VALUES (?1, ?2, ?3, ?4)",
            params![id, mod_key(m), m.archive_name, m.name],
        )?;
    }
    tx.execute("UPDATE profiles SET updated_at = datetime('now') WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

fn name_taken(e: rusqlite::Error, name: &str) -> Error {
    match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            Error::Other(format!("a profile named {name} already exists for this game version"))
        }
        e => e.into(),
    }
}

/// Save the mods enabled now as a new profile under `game_version`.
pub fn create(db: &Db, game_id: i64, game_version: &str, name: &str) -> Result<i64> {
    let name = clean_name(name)?;
    db.conn
        .execute(
            "INSERT INTO profiles (game_id, game_version, name) VALUES (?1, ?2, ?3)",
            params![game_id, game_version.trim(), name],
        )
        .map_err(|e| name_taken(e, &name))?;
    let id = db.conn.last_insert_rowid();
    store_enabled(db, id, game_id)?;
    Ok(id)
}

/// Replace a profile's mod list with the mods enabled now.
pub fn save(db: &Db, id: i64) -> Result<()> {
    let game_id = game_of(db, id)?;
    store_enabled(db, id, game_id)
}

pub fn rename(db: &Db, id: i64, name: &str) -> Result<()> {
    let name = clean_name(name)?;
    game_of(db, id)?;
    db.conn
        .execute("UPDATE profiles SET name = ?2, updated_at = datetime('now') WHERE id = ?1", params![id, name])
        .map_err(|e| name_taken(e, &name))?;
    Ok(())
}

/// Delete a profile. Installed mods are not touched.
pub fn delete(db: &Db, id: i64) -> Result<()> {
    db.conn.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
    Ok(())
}

/// What applying a profile would change.
pub fn plan(db: &Db, id: i64) -> Result<Plan> {
    let game_id = game_of(db, id)?;
    Ok(plan_for(&entries(db, id)?, &db.mods(game_id)?))
}

/// Enable the profile's mods and disable every other installed mod.
/// Disables run first so their files are out of the way; a mod that can't
/// be switched (e.g. a file conflict) is reported and the rest go ahead.
pub fn apply(inst: &Installer, id: i64) -> Result<ApplyReport> {
    let p = plan(inst.db, id)?;
    let mut r = ApplyReport { missing: p.missing, ..Default::default() };
    for mid in p.disable {
        let name = inst.db.get_mod(mid)?.name;
        match inst.disable(mid) {
            Ok(kept) => {
                r.kept_files += kept.len();
                r.disabled.push(name);
            }
            Err(e) => r.failed.push((name, e.to_string())),
        }
    }
    for mid in p.enable {
        let name = inst.db.get_mod(mid)?.name;
        match inst.enable(mid, false) {
            Ok(_) => r.enabled.push(name),
            Err(e) => r.failed.push((name, e.to_string())),
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::archive::Limits;
    use crate::db::{GameRow, NewMod};
    use crate::game::{GameInstall, Store};
    use crate::install::InstallOptions;

    fn game(db: &Db, dir: PathBuf) -> GameRow {
        std::fs::create_dir_all(dir.join("bin/x64")).unwrap();
        std::fs::write(dir.join("bin/x64/Cyberpunk2077.exe"), b"exe").unwrap();
        let gi = GameInstall {
            path: dir,
            store: Store::Manual,
            proton_prefix: None,
            build_id: Some("1".into()),
            exe_file_version: None,
            exe_product_version: Some("2.3".into()),
            frameworks: vec![],
            launch_options: None,
            warnings: vec![],
        };
        db.game(db.upsert_game(&gi).unwrap()).unwrap()
    }

    fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, d) in entries {
            z.start_file(*n, SimpleFileOptions::default()).unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap();
    }

    fn install(inst: &Installer, g: &GameRow, root: &Path, name: &str, nexus: Option<i64>, file: &str) -> i64 {
        let a = root.join(format!("{name}-{file}.zip"));
        zip_with(&a, &[(&format!("archive/pc/mod/{name}-{file}.archive"), name.as_bytes())]);
        let meta = NewMod { name: name.into(), source: if nexus.is_some() { "nexus" } else { "manual" }.into(), nexus_mod_id: nexus, ..Default::default() };
        inst.install(g, &a, InstallOptions { meta, ..Default::default() }).unwrap().mod_id
    }

    #[test]
    fn save_apply_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let db = Db::open_in_memory().unwrap();
        let g = game(&db, root.join("game"));
        let inst = Installer { db: &db, staging_root: root.join("staging"), backups_root: root.join("backups"), limits: Limits::default() };
        let a = install(&inst, &g, root, "A", Some(1), "main");
        let a_opt = install(&inst, &g, root, "A", Some(1), "optional");
        let b = install(&inst, &g, root, "B", None, "x");
        let c = install(&inst, &g, root, "C", Some(3), "x");

        // "Full": everything but A's optional file.
        inst.disable(a_opt).unwrap();
        let full = create(&db, g.id, "2.3", "Full").unwrap();
        // "Lite": only B.
        inst.disable(a).unwrap();
        inst.disable(c).unwrap();
        let lite = create(&db, g.id, "2.3", " Lite ").unwrap();
        assert!(create(&db, g.id, "2.3", "lite").unwrap_err().to_string().contains("already exists"));
        assert!(create(&db, g.id, "2.3", "  ").is_err());
        // Same name under another game version is fine.
        create(&db, g.id, "2.2", "Lite").unwrap();

        let ps = list(&db, g.id).unwrap();
        let get = |id| ps.iter().find(|p| p.id == id).unwrap().clone();
        assert!(get(lite).active);
        assert_eq!(get(lite).name, "Lite");
        let f = get(full);
        assert!(!f.active);
        assert_eq!((f.mod_count, f.to_enable, f.to_disable), (3, 2, 0));

        let r = apply(&inst, full).unwrap();
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(r.enabled.len(), 2);
        let on = |id| db.get_mod(id).unwrap().enabled();
        assert!(on(a) && on(b) && on(c) && !on(a_opt));
        assert!(root.join("game/archive/pc/mod/A-main.archive").is_file());
        assert!(!root.join("game/archive/pc/mod/A-optional.archive").exists());

        let r = apply(&inst, lite).unwrap();
        assert_eq!(r.disabled.len(), 2);
        assert!(!on(a) && on(b) && !on(c));

        // C is uninstalled: listed as missing, the rest still applies.
        inst.uninstall(c).unwrap();
        let r = apply(&inst, full).unwrap();
        assert_eq!(r.missing, vec!["C".to_string()]);
        assert!(on(a) && on(b));

        // Save overwrites with what's enabled now; rename; delete.
        save(&db, lite).unwrap();
        assert!(list(&db, g.id).unwrap().iter().find(|p| p.id == lite).unwrap().active);
        assert!(rename(&db, lite, "Full").unwrap_err().to_string().contains("already exists"));
        rename(&db, lite, "Everything").unwrap();
        delete(&db, lite).unwrap();
        assert!(plan(&db, lite).is_err());
        assert!(on(a) && on(b));
    }

    #[test]
    fn updated_mod_still_matches() {
        let db = Db::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let g = game(&db, tmp.path().join("game"));
        let m = |id: i64, archive: &str, status: &str| ModRow {
            id,
            game_id: g.id,
            name: "A".into(),
            version: None,
            source: "nexus".into(),
            nexus_mod_id: Some(7),
            nexus_file_id: None,
            archive_name: archive.into(),
            archive_sha256: String::new(),
            archive_md5: String::new(),
            game_build_id: None,
            game_version: None,
            status: status.into(),
            installed_at: String::new(),
            file_count: 0,
            category: None,
            source_ref: None,
            source_file: None,
        };
        let list = vec![Entry { key: "nexus:7".into(), archive: "A-1.0.zip".into(), name: "A".into() }];
        // The profile's file was replaced by an update: the new file counts.
        let p = plan_for(&list, &[m(1, "A-1.1.zip", "disabled")]);
        assert_eq!(p.enable, vec![1]);
        // The exact file is installed: only it is enabled.
        let p = plan_for(&list, &[m(1, "A-1.0.zip", "disabled"), m(2, "A-opt.zip", "installed")]);
        assert_eq!((p.enable, p.disable), (vec![1], vec![2]));
    }
}
