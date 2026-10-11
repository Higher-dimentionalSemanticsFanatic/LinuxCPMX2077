//! Archive load order: the order the game reads `.archive` files in, which
//! decides which mod's copy of a shared resource is used.
//!
//! - `archive/pc/mod/*.archive` load first, sorted by file name byte-wise
//!   (ASCII: `!` and `_` and uppercase before lowercase), or in the order of
//!   `archive/pc/mod/modlist.txt` when that file exists (archives it doesn't
//!   list follow, by name).
//! - REDmod archives (`mods/<folder>/archives/*.archive`) load after every
//!   archive above, by folder name, then file name.
//! - The first archive containing a resource wins.
//!
//! Source: the RED modding wiki's load-order page (CDPR-Modding-Documentation).
//!
//! Changing the order writes `modlist.txt` (`set`); the app remembers what
//! it wrote so it only ever removes its own file (`reset`), and a
//! `modlist.txt` that was there before is kept aside and put back.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::db::{Db, GameRow};
use crate::install::resolve_ci;
use crate::{Error, Result};

/// How the archive order was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// File-name order, the game's default.
    Alphabetical,
    /// `archive/pc/mod/modlist.txt`.
    Modlist,
}

/// Where an archive sits in the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Archive,
    Redmod,
}

#[derive(Debug, Clone, Serialize)]
pub struct Slot {
    /// 1-based load position.
    pub pos: usize,
    /// Path inside the game folder.
    pub path: String,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadOrder {
    pub via: Via,
    pub slots: Vec<Slot>,
}

impl LoadOrder {
    /// Load position by lowercased game path.
    pub fn positions(&self) -> HashMap<String, usize> {
        self.slots.iter().map(|s| (s.path.to_lowercase(), s.pos)).collect()
    }
}

/// Which kind of loaded archive a game path is, if any.
pub fn source_of(path: &str) -> Option<Source> {
    let p = path.to_lowercase();
    if !p.ends_with(".archive") {
        return None;
    }
    let parts: Vec<&str> = p.split('/').collect();
    match parts.as_slice() {
        ["archive", "pc", "mod", _] => Some(Source::Archive),
        ["mods", _, "archives", _] => Some(Source::Redmod),
        _ => None,
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Sort loaded archive paths into load order; paths the game doesn't load
/// (`source_of` is None) are dropped. `modlist` holds file names.
pub fn sort(paths: &[String], modlist: Option<&[String]>) -> Vec<String> {
    let rank: HashMap<String, usize> =
        modlist.unwrap_or_default().iter().enumerate().map(|(i, n)| (n.to_lowercase(), i)).collect();
    let mut keyed: Vec<(Source, usize, Vec<u8>, &String)> = paths
        .iter()
        .filter_map(|p| {
            let src = source_of(p)?;
            Some(match src {
                Source::Archive => {
                    let name = file_name(p);
                    (src, rank.get(&name.to_lowercase()).copied().unwrap_or(usize::MAX), name.as_bytes().to_vec(), p)
                }
                // Folder, then file: "mods/<folder>/archives/<file>".
                Source::Redmod => {
                    let mut parts = p.split('/').skip(1);
                    let folder = parts.next().unwrap_or("");
                    let key = [folder.as_bytes(), &[0], file_name(p).as_bytes()].concat();
                    (src, 0, key, p)
                }
            })
        })
        .collect();
    keyed.sort();
    keyed.dedup_by(|a, b| a.3.eq_ignore_ascii_case(b.3));
    keyed.into_iter().map(|k| k.3.clone()).collect()
}

/// `archive/pc/mod/modlist.txt`, one archive file name per line.
pub fn read_modlist(game_dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(resolve_ci(game_dir, "archive/pc/mod/modlist.txt")).ok()?;
    Some(text.lines().map(|l| l.trim().trim_start_matches('\u{feff}').to_string()).filter(|l| !l.is_empty()).collect())
}

/// The order of every archive in the game folder, whoever installed it.
pub fn effective(game_dir: &Path) -> LoadOrder {
    let mut paths = Vec::new();
    let list = |rel: &str| -> Vec<String> {
        std::fs::read_dir(resolve_ci(game_dir, rel))
            .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    for name in list("archive/pc/mod") {
        paths.push(format!("archive/pc/mod/{name}"));
    }
    for folder in list("mods") {
        for name in list(&format!("mods/{folder}/archives")) {
            paths.push(format!("mods/{folder}/archives/{name}"));
        }
    }
    let modlist = read_modlist(game_dir);
    let via = if modlist.is_some() { Via::Modlist } else { Via::Alphabetical };
    let slots = sort(&paths, modlist.as_deref())
        .into_iter()
        .enumerate()
        .map(|(i, path)| Slot { pos: i + 1, source: source_of(&path).unwrap_or(Source::Archive), path })
        .collect();
    LoadOrder { via, slots }
}

/// Per archive (lowercased path): how many shared resources it wins and
/// loses against other archives. `touches` are (archive path, resource key)
/// pairs; archives without a position are left out.
pub fn contests<'a>(touches: impl IntoIterator<Item = (&'a str, &'a str)>, pos: &HashMap<String, usize>) -> HashMap<String, (usize, usize)> {
    let mut by_key: HashMap<&str, Vec<(usize, String)>> = HashMap::new();
    for (file, key) in touches {
        let lower = file.to_lowercase();
        if let Some(&p) = pos.get(&lower) {
            let v = by_key.entry(key).or_default();
            if !v.iter().any(|(_, f)| *f == lower) {
                v.push((p, lower));
            }
        }
    }
    let mut out: HashMap<String, (usize, usize)> = HashMap::new();
    for (_, mut v) in by_key.into_iter().filter(|(_, v)| v.len() > 1) {
        v.sort();
        for (i, (_, f)) in v.into_iter().enumerate() {
            let e = out.entry(f).or_default();
            if i == 0 { e.0 += 1 } else { e.1 += 1 }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Changing the order

const MODLIST: &str = "archive/pc/mod/modlist.txt";

fn owned_key(game_id: i64) -> String {
    format!("load_order.owned.{game_id}")
}

fn modlist_path(game_dir: &Path) -> std::path::PathBuf {
    resolve_ci(game_dir, "archive/pc/mod").join("modlist.txt")
}

fn user_backup(backups_root: &Path, game_id: i64) -> std::path::PathBuf {
    backups_root.join(format!("modlist-{game_id}.txt"))
}

/// `modlist.txt` exists and is the one CPMX2077 wrote.
pub fn managed(db: &Db, game: &GameRow) -> Result<bool> {
    let Some(owned) = db.get_setting(&owned_key(game.id))? else { return Ok(false) };
    Ok(std::fs::read_to_string(modlist_path(Path::new(&game.path))).is_ok_and(|t| t == owned))
}

/// The order with each archive's mod, for the Load order view.
#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub via: Via,
    /// `modlist.txt` was written by CPMX2077.
    pub managed: bool,
    pub slots: Vec<ViewSlot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewSlot {
    pub pos: usize,
    pub path: String,
    pub name: String,
    pub source: Source,
    pub mod_id: Option<i64>,
    pub mod_name: Option<String>,
}

pub fn view(db: &Db, game: &GameRow) -> Result<View> {
    let order = effective(Path::new(&game.path));
    let mut owner: HashMap<String, (i64, String)> = HashMap::new();
    for m in db.mods(game.id)?.into_iter().filter(|m| m.enabled()) {
        for f in db.mod_files(m.id)? {
            if source_of(&f.rel_path).is_some() {
                owner.insert(f.rel_path.to_lowercase(), (m.id, m.name.clone()));
            }
        }
    }
    let slots = order
        .slots
        .into_iter()
        .map(|s| {
            let o = owner.get(&s.path.to_lowercase());
            ViewSlot { pos: s.pos, name: file_name(&s.path).to_string(), source: s.source, mod_id: o.map(|o| o.0), mod_name: o.map(|o| o.1.clone()), path: s.path }
        })
        .collect();
    Ok(View { via: order.via, managed: managed(db, game)?, slots })
}

/// Write `names` (archive file names in `archive/pc/mod`, first loaded
/// first) as `modlist.txt`. Archives in the folder that `names` leaves out
/// are appended by name, so every archive is listed. A `modlist.txt` that
/// CPMX2077 didn't write is moved to `backups_root` first.
pub fn set(db: &Db, game: &GameRow, backups_root: &Path, names: &[String]) -> Result<()> {
    let game_dir = Path::new(&game.path);
    let present: Vec<String> = effective(game_dir).slots.into_iter().filter(|s| s.source == Source::Archive).map(|s| file_name(&s.path).to_string()).collect();
    let mut out: Vec<String> = Vec::new();
    for n in names {
        if n.contains(['/', '\\', '\r', '\n']) || !n.to_lowercase().ends_with(".archive") {
            return Err(Error::Other(format!("not an archive file name: {n}")));
        }
        // Archives removed since the order was made are skipped.
        let Some(real) = present.iter().find(|p| p.eq_ignore_ascii_case(n)) else { continue };
        if !out.iter().any(|o| o.eq_ignore_ascii_case(real)) {
            out.push(real.clone());
        }
    }
    let mut rest: Vec<&String> = present.iter().filter(|p| !out.iter().any(|o| o.eq_ignore_ascii_case(p))).collect();
    rest.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    out.extend(rest.into_iter().cloned());

    let path = modlist_path(game_dir);
    if path.exists() && !managed(db, game)? {
        std::fs::create_dir_all(backups_root)?;
        std::fs::copy(&path, user_backup(backups_root, game.id))?;
    }
    let text: String = out.iter().map(|n| format!("{n}\r\n")).collect();
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().ok_or_else(|| Error::Other(format!("no folder for {MODLIST}")))?)?;
    std::io::Write::write_all(&mut tmp, text.as_bytes())?;
    tmp.persist(&path).map_err(|e| Error::Other(format!("writing {MODLIST}: {e}")))?;
    db.set_setting(&owned_key(game.id), &text)?;
    Ok(())
}

/// Back to the game's default order: remove the `modlist.txt` CPMX2077
/// wrote and put back one that was there before. A `modlist.txt` someone
/// else wrote is left alone. Returns whether anything changed.
pub fn reset(db: &Db, game: &GameRow, backups_root: &Path) -> Result<bool> {
    if !managed(db, game)? {
        return Ok(false);
    }
    let path = modlist_path(Path::new(&game.path));
    std::fs::remove_file(&path)?;
    let backup = user_backup(backups_root, game.id);
    if backup.is_file() {
        std::fs::copy(&backup, &path)?;
        std::fs::remove_file(&backup)?;
    }
    db.set_setting(&owned_key(game.id), "")?;
    Ok(true)
}

/// The archive order to store with a profile: the managed `modlist.txt`'s
/// names, or None for the game's default order.
pub fn current_managed(db: &Db, game: &GameRow) -> Result<Option<Vec<String>>> {
    Ok(if managed(db, game)? { read_modlist(Path::new(&game.path)) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn ascii_order_then_redmods() {
        let paths = s(&[
            "mods/b/archives/x.archive",
            "archive/pc/mod/a.archive",
            "archive/pc/mod/_00_patch.archive",
            "archive/pc/mod/B.archive",
            "archive/pc/mod/!first.archive",
            "mods/A/archives/z.archive",
            "archive/pc/mod/readme.txt",
            "archive/pc/mod/sub/deep.archive",
        ]);
        assert_eq!(
            sort(&paths, None),
            s(&[
                "archive/pc/mod/!first.archive",
                "archive/pc/mod/B.archive",
                "archive/pc/mod/_00_patch.archive",
                "archive/pc/mod/a.archive",
                "mods/A/archives/z.archive",
                "mods/b/archives/x.archive",
            ])
        );
    }

    #[test]
    fn modlist_first_then_rest() {
        let paths = s(&["archive/pc/mod/a.archive", "archive/pc/mod/b.archive", "archive/pc/mod/c.archive", "mods/m/archives/r.archive"]);
        let ml = s(&["c.archive", "A.archive"]);
        assert_eq!(
            sort(&paths, Some(&ml)),
            s(&["archive/pc/mod/c.archive", "archive/pc/mod/a.archive", "archive/pc/mod/b.archive", "mods/m/archives/r.archive"])
        );
    }

    #[test]
    fn reads_folder_and_counts_contests() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path();
        std::fs::create_dir_all(g.join("archive/pc/mod")).unwrap();
        std::fs::create_dir_all(g.join("mods/Rm/archives")).unwrap();
        for f in ["archive/pc/mod/zz.archive", "archive/pc/mod/aa.archive", "mods/Rm/archives/r.archive"] {
            std::fs::write(g.join(f), b"").unwrap();
        }
        let lo = effective(g);
        assert_eq!(lo.via, Via::Alphabetical);
        let order: Vec<(usize, &str)> = lo.slots.iter().map(|s| (s.pos, s.path.as_str())).collect();
        assert_eq!(order, vec![(1, "archive/pc/mod/aa.archive"), (2, "archive/pc/mod/zz.archive"), (3, "mods/Rm/archives/r.archive")]);

        std::fs::write(g.join("archive/pc/mod/modlist.txt"), "zz.archive\r\naa.archive\r\n").unwrap();
        let lo = effective(g);
        assert_eq!(lo.via, Via::Modlist);
        assert_eq!(lo.slots[0].path, "archive/pc/mod/zz.archive");

        let pos = lo.positions();
        let t = [
            ("archive/pc/mod/zz.archive", "1"),
            ("archive/pc/mod/aa.archive", "1"),
            ("mods/Rm/archives/r.archive", "1"),
            ("archive/pc/mod/aa.archive", "2"),
            ("mods/Rm/archives/r.archive", "2"),
            ("archive/pc/mod/zz.archive", "3"),
        ];
        let c = contests(t, &pos);
        assert_eq!(c["archive/pc/mod/zz.archive"], (1, 0));
        assert_eq!(c["archive/pc/mod/aa.archive"], (1, 1));
        assert_eq!(c["mods/rm/archives/r.archive"], (0, 2));
    }

    #[test]
    fn set_and_reset_keep_a_users_modlist() {
        use crate::game::{GameInstall, Store};
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path().join("game");
        let backups = dir.path().join("backups");
        std::fs::create_dir_all(g.join("archive/pc/mod")).unwrap();
        for f in ["a.archive", "B.archive", "c.archive"] {
            std::fs::write(g.join("archive/pc/mod").join(f), b"").unwrap();
        }
        std::fs::write(g.join("archive/pc/mod/modlist.txt"), "mine\n").unwrap();
        let db = Db::open_in_memory().unwrap();
        let gi = GameInstall { path: g.clone(), store: Store::Manual, proton_prefix: None, build_id: None, exe_file_version: None, exe_product_version: None, frameworks: vec![], launch_options: None, warnings: vec![] };
        let game = db.game(db.upsert_game(&gi).unwrap()).unwrap();
        assert!(!managed(&db, &game).unwrap());
        assert!(!reset(&db, &game, &backups).unwrap(), "someone else's modlist.txt stays");

        assert!(set(&db, &game, &backups, &s(&["../x.archive"])).is_err());
        set(&db, &game, &backups, &s(&["c.archive", "gone.archive", "A.archive"])).unwrap();
        assert!(managed(&db, &game).unwrap());
        assert_eq!(read_modlist(&g).unwrap(), s(&["c.archive", "a.archive", "B.archive"]));
        assert_eq!(effective(&g).slots[0].path, "archive/pc/mod/c.archive");
        assert_eq!(current_managed(&db, &game).unwrap().unwrap().len(), 3);

        assert!(reset(&db, &game, &backups).unwrap());
        assert_eq!(std::fs::read_to_string(g.join("archive/pc/mod/modlist.txt")).unwrap(), "mine\n");
        assert!(!managed(&db, &game).unwrap());
    }
}
