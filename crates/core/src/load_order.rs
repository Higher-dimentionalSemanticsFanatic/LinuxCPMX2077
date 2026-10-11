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

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::install::resolve_ci;

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
}
