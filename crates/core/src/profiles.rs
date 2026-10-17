//! Named mod profiles, filed under the game version they were made for. A
//! profile is a list of mods; applying it enables those mods and disables
//! every other installed mod, in the same game folder.
//!
//! Mods are matched by [`mod_key`] so a profile still finds a mod after it
//! was updated. Where one mod has several files installed (a Nexus main file
//! plus an optional file), the archive name picks out the ones in the
//! profile.
//!
//! Installing a modpack (Nexus collection) can switch to a profile of its
//! own, made from the collection's mod list: see [`switch_to_pack`].

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

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
    /// Slug of the Nexus collection the profile was made for.
    pub collection: Option<String>,
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
        "SELECT id, game_version, name, created_at, updated_at, collection_slug FROM profiles
         WHERE game_id = ?1 ORDER BY game_version, name COLLATE NOCASE",
    )?;
    type Row = (i64, String, String, String, String, Option<String>);
    let rows: Vec<Row> = st
        .query_map([game_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, game_version, name, created_at, updated_at, collection) in rows {
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
            collection,
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
    let order = match crate::load_order::current_managed(db, &db.game(game_id)?)? {
        Some(names) => serde_json::to_string(&names)?,
        None => String::new(),
    };
    tx.execute("UPDATE profiles SET updated_at = datetime('now'), load_order = ?2 WHERE id = ?1", params![id, order])?;
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
    // The archive load order saved with the profile.
    let game = inst.db.game(game_of(inst.db, id)?)?;
    let order: Option<String> = inst.db.conn.query_row("SELECT load_order FROM profiles WHERE id = ?1", [id], |r| r.get(0))?;
    match order.as_deref() {
        None => {}
        Some("") => {
            crate::load_order::reset(inst.db, &game, &inst.backups_root)?;
        }
        Some(json) => {
            let names: Vec<String> = serde_json::from_str(json)?;
            crate::load_order::set(inst.db, &game, &inst.backups_root, &names)?;
        }
    }
    Ok(r)
}

/// The stock game: disable every enabled mod of the game. Files the app
/// doesn't track (mods copied in by hand) stay where they are.
pub fn apply_stock(inst: &Installer, game_id: i64) -> Result<ApplyReport> {
    let mut r = ApplyReport::default();
    for m in inst.db.mods(game_id)?.into_iter().filter(|m| m.status == "installed") {
        match inst.disable(m.id) {
            Ok(kept) => {
                r.kept_files += kept.len();
                r.disabled.push(m.name);
            }
            Err(e) => r.failed.push((m.name, e.to_string())),
        }
    }
    let game = inst.db.game(game_id)?;
    crate::load_order::reset(inst.db, &game, &inst.backups_root)?;
    Ok(r)
}

// ---------------------------------------------------------------------------
// Modpack profiles

/// One mod of a modpack, as the Modpacks tab sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct PackMod {
    pub mod_id: i64,
    #[serde(default)]
    pub file_name: Option<String>,
    pub mod_name: String,
}

/// What switching to a modpack's profile would change.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PackPlan {
    #[serde(flatten)]
    pub plan: Plan,
    /// The modpack's profile when it already has one.
    pub profile: Option<String>,
    /// The name the mods enabled now are saved under first, when no
    /// profile holds them.
    pub save_before: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackSwitch {
    pub profile_id: i64,
    pub profile: String,
    pub saved_before: Option<String>,
    #[serde(flatten)]
    pub report: ApplyReport,
}

fn pack_profile(db: &Db, game_id: i64, slug: &str) -> Result<Option<(i64, String)>> {
    Ok(db
        .conn
        .query_row(
            "SELECT id, name FROM profiles WHERE game_id = ?1 AND collection_slug = ?2 ORDER BY id DESC LIMIT 1",
            params![game_id, slug],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// `base`, or `base 2`, `base 3`, ... when a profile under `game_version`
/// already has the name; cut to [`MAX_NAME`].
fn unique_name(db: &Db, game_id: i64, game_version: &str, base: &str) -> Result<String> {
    let base = clean_name(&base.chars().take(MAX_NAME - 4).collect::<String>())?;
    let taken = |n: &str| -> Result<bool> {
        Ok(db
            .conn
            .query_row(
                "SELECT 1 FROM profiles WHERE game_id = ?1 AND game_version = ?2 AND name = ?3 COLLATE NOCASE",
                params![game_id, game_version, n],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    };
    let mut name = base.clone();
    let mut i = 2;
    while taken(&name)? {
        name = format!("{base} {i}");
        i += 1;
    }
    Ok(name)
}

/// The modpack's mods, the ones its profile already lists, and the
/// frameworks enabled now (CET, RED4ext, ... installed from GitHub have no
/// Nexus id, so the collection's entries would not keep them on).
fn pack_entries(db: &Db, game_id: i64, existing: Option<i64>, wanted: &[PackMod]) -> Result<Vec<Entry>> {
    let mut out: BTreeSet<Entry> = wanted
        .iter()
        .map(|w| Entry { key: format!("nexus:{}", w.mod_id), archive: w.file_name.clone().unwrap_or_default(), name: w.mod_name.clone() })
        .collect();
    if let Some(id) = existing {
        out.extend(entries(db, id)?);
    }
    for m in db.mods(game_id)?.iter().filter(|m| m.enabled() && crate::dependencies::framework_of(m).is_some()) {
        out.insert(Entry { key: mod_key(m), archive: m.archive_name.clone(), name: m.name.clone() });
    }
    Ok(out.into_iter().collect())
}

/// The name to save the enabled mods under before switching, when they
/// match no profile.
fn before_name(db: &Db, game_id: i64, game_version: &str, pack: &str) -> Result<Option<String>> {
    let any_enabled = db.mods(game_id)?.iter().any(ModRow::enabled);
    if !any_enabled || list(db, game_id)?.iter().any(|p| p.active) {
        return Ok(None);
    }
    Ok(Some(unique_name(db, game_id, game_version.trim(), &format!("Before {pack}"))?))
}

/// What [`switch_to_pack`] would do, without changing anything.
pub fn pack_plan(db: &Db, game_id: i64, game_version: &str, slug: &str, pack: &str, wanted: &[PackMod]) -> Result<PackPlan> {
    let existing = pack_profile(db, game_id, slug)?;
    let list = pack_entries(db, game_id, existing.as_ref().map(|e| e.0), wanted)?;
    Ok(PackPlan {
        plan: plan_for(&list, &db.mods(game_id)?),
        profile: existing.map(|e| e.1),
        save_before: before_name(db, game_id, game_version, pack)?,
    })
}

/// Switch to the modpack's own profile before its mods are installed: save
/// the mods enabled now as a profile when none holds them, create the
/// modpack's profile (or add `wanted` to the one it has), and apply it.
/// Mods still to be installed are listed as missing; once installed they
/// count as part of the profile.
pub fn switch_to_pack(inst: &Installer, game_id: i64, game_version: &str, slug: &str, pack: &str, wanted: &[PackMod]) -> Result<PackSwitch> {
    let db = inst.db;
    let saved_before = before_name(db, game_id, game_version, pack)?;
    if let Some(name) = &saved_before {
        create(db, game_id, game_version, name)?;
    }
    let existing = pack_profile(db, game_id, slug)?;
    let list = pack_entries(db, game_id, existing.as_ref().map(|e| e.0), wanted)?;
    let (id, profile) = match existing {
        Some(e) => e,
        None => {
            let name = unique_name(db, game_id, game_version.trim(), pack)?;
            // The game's default archive order: the collection's own order
            // is not installed here.
            db.conn.execute(
                "INSERT INTO profiles (game_id, game_version, name, load_order, collection_slug) VALUES (?1, ?2, ?3, '', ?4)",
                params![game_id, game_version.trim(), name, slug],
            )?;
            (db.conn.last_insert_rowid(), name)
        }
    };
    let tx = db.conn.unchecked_transaction()?;
    tx.execute("DELETE FROM profile_mods WHERE profile_id = ?1", [id])?;
    for e in &list {
        tx.execute(
            "INSERT OR IGNORE INTO profile_mods (profile_id, mod_key, archive_name, name) VALUES (?1, ?2, ?3, ?4)",
            params![id, e.key, e.archive, e.name],
        )?;
    }
    tx.execute("UPDATE profiles SET updated_at = datetime('now') WHERE id = ?1", [id])?;
    tx.commit()?;
    let report = apply(inst, id)?;
    Ok(PackSwitch { profile_id: id, profile, saved_before, report })
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

        // Stock disables everything; the full profile brings it back.
        let r = apply_stock(&inst, g.id).unwrap();
        assert_eq!(r.disabled.len(), 2);
        assert!(!on(a) && !on(b) && !root.join("game/archive/pc/mod/A-main.archive").exists());
        apply(&inst, full).unwrap();
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

    #[test]
    fn profiles_carry_the_archive_order() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let db = Db::open_in_memory().unwrap();
        let g = game(&db, root.join("game"));
        let inst = Installer { db: &db, staging_root: root.join("staging"), backups_root: root.join("backups"), limits: Limits::default() };
        install(&inst, &g, root, "A", None, "x");
        install(&inst, &g, root, "B", None, "x");
        let default = create(&db, g.id, "2.3", "Default").unwrap();
        crate::load_order::set(&db, &g, &inst.backups_root, &["B-x.archive".to_string()]).unwrap();
        let ordered = create(&db, g.id, "2.3", "Ordered").unwrap();
        // A profile from before load order could be changed leaves it alone.
        db.conn.execute("UPDATE profiles SET load_order = NULL WHERE id = ?1", [default]).unwrap();
        apply(&inst, default).unwrap();
        assert!(crate::load_order::managed(&db, &g).unwrap());

        db.conn.execute("UPDATE profiles SET load_order = '' WHERE id = ?1", [default]).unwrap();
        apply(&inst, default).unwrap();
        assert!(!crate::load_order::managed(&db, &g).unwrap());
        assert!(!Path::new(&g.path).join("archive/pc/mod/modlist.txt").exists());

        apply(&inst, ordered).unwrap();
        let ml = crate::load_order::read_modlist(Path::new(&g.path)).unwrap();
        assert_eq!(ml, vec!["B-x.archive".to_string(), "A-x.archive".to_string()]);
        apply_stock(&inst, g.id).unwrap();
        assert!(!crate::load_order::managed(&db, &g).unwrap());
    }

    #[test]
    fn modpack_gets_its_own_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let db = Db::open_in_memory().unwrap();
        let g = game(&db, root.join("game"));
        let inst = Installer { db: &db, staging_root: root.join("staging"), backups_root: root.join("backups"), limits: Limits::default() };
        let own = install(&inst, &g, root, "Own", Some(1), "x");
        let shared = install(&inst, &g, root, "Shared", Some(2), "x");
        let off = install(&inst, &g, root, "Off", Some(3), "x");
        inst.disable(off).unwrap();
        // A framework from GitHub: no Nexus id, stays on.
        let a = root.join("cet.zip");
        zip_with(&a, &[("bin/x64/plugins/cyber_engine_tweaks.asi", b"cet")]);
        let meta = NewMod { name: "Cyber Engine Tweaks".into(), source: "github".into(), source_ref: Some("maximegmd/CyberEngineTweaks".into()), ..Default::default() };
        let cet = inst.install(&g, &a, InstallOptions { meta, ..Default::default() }).unwrap().mod_id;

        let pm = |id: i64, name: &str| PackMod { mod_id: id, file_name: None, mod_name: name.into() };
        let wanted = vec![pm(2, "Shared"), pm(3, "Off"), pm(9, "New")];
        let p = pack_plan(&db, g.id, "2.3", "abc123", "Pack", &wanted).unwrap();
        assert_eq!((p.plan.enable, p.plan.disable, p.plan.missing.clone()), (vec![off], vec![own], vec!["New".to_string()]));
        assert_eq!((p.profile, p.save_before.as_deref()), (None, Some("Before Pack")));

        let s = switch_to_pack(&inst, g.id, "2.3", "abc123", "Pack", &wanted).unwrap();
        assert_eq!((s.profile.as_str(), s.saved_before.as_deref()), ("Pack", Some("Before Pack")));
        assert!(s.report.failed.is_empty(), "{:?}", s.report.failed);
        let on = |id| db.get_mod(id).unwrap().enabled();
        assert!(!on(own) && on(shared) && on(off) && on(cet));

        // The new mod lands: the pack's profile is active.
        install(&inst, &g, root, "New", Some(9), "x");
        let ps = list(&db, g.id).unwrap();
        let pack = ps.iter().find(|p| p.id == s.profile_id).unwrap();
        assert!(pack.active && pack.missing.is_empty());
        assert_eq!(pack.collection.as_deref(), Some("abc123"));

        // Back to the saved set, then a second install reuses the profile
        // and keeps what it listed.
        let before = ps.iter().find(|p| p.name == "Before Pack").unwrap();
        apply(&inst, before.id).unwrap();
        assert!(on(own) && on(shared) && !on(off));
        let s2 = switch_to_pack(&inst, g.id, "2.3", "abc123", "Pack", &[pm(4, "Extra")]).unwrap();
        assert_eq!((s2.profile_id, s2.saved_before), (s.profile_id, None));
        assert!(!on(own) && on(off) && on(cet));
        assert_eq!(s2.report.missing, vec!["Extra".to_string()]);
    }
}
