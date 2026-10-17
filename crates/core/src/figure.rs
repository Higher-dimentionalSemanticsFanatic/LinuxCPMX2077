//! Installed mods › Canvas › Figure: which body or clothing slot a mod's
//! files point at. Read from the mod's own files, deterministic:
//!
//! - TweakXL item types (`$base: Items.GenericOuterChestClothing`, ...);
//! - depot paths found in TweakXL / ArchiveXL text (`base\characters\...`);
//! - depot paths inside its `.archive` files, when WolvenKit listed them
//!   (cached by file hash, see [`cached_listing`]).
//!
//! The UI adds the mod's tags and name words on top (`ui/mods_figure.js`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Result;

/// Slot keys shared with the UI.
pub const SLOTS: [&str; 15] =
    ["hair", "eyes", "face", "makeup", "skin", "tattoos", "body", "head", "facewear", "outer", "inner", "acc", "legs", "feet", "outfit"];

/// Text files bigger than this are skipped (tweaks and .xl files are small).
const MAX_TEXT: u64 = 2 * 1024 * 1024;

/// Slot of one depot path, by the game's folder layout and garment mesh
/// prefixes (`h0_` head, `f1_` face, `t1_` inner torso, `t2_` outer torso,
/// `l1_` legs, `s1_` feet).
pub fn slot_of_path(path: &str) -> Option<&'static str> {
    let p = path.to_ascii_lowercase().replace('/', "\\");
    if !p.contains("characters\\") {
        return None;
    }
    let name = p.rsplit('\\').next().unwrap_or("");
    if p.contains("\\garment\\") || p.contains("\\garments\\") {
        let prefix = |s: &str| name.starts_with(s) || name.contains(&format!("_{s}"));
        return if prefix("h0_") || prefix("h1_") {
            Some("head")
        } else if prefix("f1_") {
            Some("facewear")
        } else if prefix("t2_") {
            Some("outer")
        } else if prefix("t1_") || prefix("t0_") {
            Some("inner")
        } else if prefix("l1_") {
            Some("legs")
        } else if prefix("s1_") {
            Some("feet")
        } else {
            None
        };
    }
    if p.contains("tattoo") {
        Some("tattoos")
    } else if p.contains("makeup") || p.contains("lipstick") || p.contains("eyeliner") {
        Some("makeup")
    } else if p.contains("\\hair\\") || p.contains("\\hairs\\") || p.contains("\\beard") || p.contains("\\eyebrow") {
        Some("hair")
    } else if p.contains("\\eyes\\") || p.contains("\\eye\\") || name.starts_with("eye") || name.contains("_eye") {
        Some("eyes")
    } else if p.contains("\\skin") || name.contains("skin") {
        Some("skin")
    } else if p.contains("\\head\\") || p.contains("\\heads\\") {
        Some("face")
    } else if p.contains("\\body\\") || p.contains("_bodies\\") || p.contains("\\bodies\\") {
        Some("body")
    } else if p.contains("\\accessories\\") || p.contains("\\jewelry") || p.contains("\\nails\\") {
        Some("acc")
    } else {
        None
    }
}

/// Clothing slots named by TweakXL item types in a tweak file.
pub fn slots_of_tweak(src: &str) -> BTreeSet<&'static str> {
    const TYPES: [(&str, &str); 7] = [
        ("GenericHeadClothing", "head"),
        ("GenericFaceClothing", "facewear"),
        ("GenericOuterChestClothing", "outer"),
        ("GenericInnerChestClothing", "inner"),
        ("GenericLegClothing", "legs"),
        ("GenericFootClothing", "feet"),
        ("Items.Outfit", "outfit"),
    ];
    TYPES.iter().filter(|(t, _)| src.contains(t)).map(|&(_, s)| s).collect()
}

/// Depot paths (`base\...` or `base/...`) quoted or listed in a text file.
fn paths_in_text(src: &str) -> impl Iterator<Item = &str> {
    src.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '[' | ']' | '{' | '}' | ':'))
        .filter(|t| t.len() > 5 && (t.starts_with("base\\") || t.starts_with("base/") || t.starts_with("ep1\\") || t.starts_with("ep1/")))
}

/// Slots for one mod from its files: `(game-relative target, staged copy)`,
/// plus depot paths WolvenKit listed from its archives.
pub fn slots_of_mod(files: &[(String, PathBuf)], archive_paths: &[String]) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for (rel, staged) in files {
        let ext = Path::new(rel).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !matches!(ext.as_str(), "yaml" | "yml" | "tweak" | "xl") {
            continue;
        }
        if std::fs::metadata(staged).map(|m| m.len() > MAX_TEXT).unwrap_or(true) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(staged) else { continue };
        out.extend(slots_of_tweak(&src));
        out.extend(paths_in_text(&src).filter_map(slot_of_path));
    }
    out.extend(archive_paths.iter().filter_map(|p| slot_of_path(p)));
    out
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Hints {
    /// Mod id -> slots its files point at.
    pub slots: BTreeMap<i64, Vec<String>>,
    /// Mods with `.archive` files, and how many of those have a WolvenKit
    /// listing cached.
    pub with_archives: usize,
    pub archives_read: usize,
}

/// Cached WolvenKit listing of an archive, by its SHA-256.
pub fn cached_listing(cache: &Path, sha256: &str) -> Option<Vec<String>> {
    if sha256.is_empty() || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let text = std::fs::read_to_string(cache.join(format!("{sha256}.txt"))).ok()?;
    Some(text.lines().map(String::from).collect())
}

pub fn store_listing(cache: &Path, sha256: &str, paths: &[String]) -> Result<()> {
    if sha256.is_empty() || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(());
    }
    std::fs::create_dir_all(cache)?;
    std::fs::write(cache.join(format!("{sha256}.txt")), paths.join("\n"))?;
    Ok(())
}

pub fn cache_dir() -> Result<PathBuf> {
    Ok(crate::paths::data_dir()?.join("figure-archives"))
}

/// The `.archive` files of a mod: (staged copy, SHA-256).
pub fn mod_archives(db: &crate::db::Db, staging_root: &Path, mod_id: i64) -> Result<Vec<(PathBuf, String)>> {
    Ok(db
        .mod_files(mod_id)?
        .into_iter()
        .filter(|f| f.rel_path.to_ascii_lowercase().ends_with(".archive"))
        .map(|f| (staging_root.join(mod_id.to_string()).join(&f.staged_path), f.sha256))
        .collect())
}

/// Slot hints for every mod of a game, from files and cached listings.
pub fn hints_for_game(db: &crate::db::Db, staging_root: &Path, cache: &Path, game_id: i64) -> Result<Hints> {
    let mut hints = Hints::default();
    for m in db.mods(game_id)? {
        let files: Vec<(String, PathBuf)> = db
            .mod_files(m.id)?
            .into_iter()
            .map(|f| (f.rel_path, staging_root.join(m.id.to_string()).join(&f.staged_path)))
            .collect();
        let archives = mod_archives(db, staging_root, m.id)?;
        let mut listed = Vec::new();
        let mut all_read = !archives.is_empty();
        for (_, sha) in &archives {
            match cached_listing(cache, sha) {
                Some(l) => listed.extend(l),
                None => all_read = false,
            }
        }
        if !archives.is_empty() {
            hints.with_archives += 1;
            hints.archives_read += usize::from(all_read);
        }
        let slots = slots_of_mod(&files, &listed);
        if !slots.is_empty() {
            hints.slots.insert(m.id, slots.into_iter().map(String::from).collect());
        }
    }
    Ok(hints)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garment_prefixes_pick_clothing_slots() {
        assert_eq!(slot_of_path(r"base\characters\garment\player_equipment\torso\t2_084_jacket__short_sleeves\t2_084_pwa_jacket.mesh"), Some("outer"));
        assert_eq!(slot_of_path("base/characters/garment/gang_nomad/legs/l1_001_pants/l1_001_ma_pants.mesh"), Some("legs"));
        assert_eq!(slot_of_path(r"base\characters\garment\citizen_casual\feet\s1_002_shoe\s1_002_wa_shoe.mesh"), Some("feet"));
        assert_eq!(slot_of_path(r"base\characters\garment\player_equipment\head\h1_001_cap\h1_001_pma_cap.mesh"), Some("head"));
        assert_eq!(slot_of_path(r"base\characters\garment\player_equipment\face\f1_002_glasses\f1_002_pwa_glasses.mesh"), Some("facewear"));
        assert_eq!(slot_of_path(r"base\characters\garment\player_equipment\torso\t1_024_tshirt\t1_024_pwa_tshirt.mesh"), Some("inner"));
        assert_eq!(slot_of_path(r"base\characters\garment\misc\readme.json"), None);
    }

    #[test]
    fn character_folders_pick_body_slots() {
        assert_eq!(slot_of_path(r"base\characters\common\hair\hh_033_wa__ponytail\hh_033_wa__ponytail.mesh"), Some("hair"));
        assert_eq!(slot_of_path(r"base\characters\common\eyes\eye_iris_d01.xbm"), Some("eyes"));
        assert_eq!(slot_of_path(r"base\characters\head\player_base_heads\player_female_average\h0_000_pwa_c__basehead.mesh"), Some("face"));
        assert_eq!(slot_of_path(r"base\characters\common\skin\character_mat_instance\female\body\skin_01.mi"), Some("skin"));
        assert_eq!(slot_of_path(r"base\characters\common\tattoos\tattoo_arm_01.xbm"), Some("tattoos"));
        assert_eq!(slot_of_path(r"base\characters\common\makeup\lips\makeup_lips_01.xbm"), Some("makeup"));
        assert_eq!(slot_of_path(r"base\characters\common\player_base_bodies\player_female_average\t0_000_pwa_base__full.mesh"), Some("body"));
        assert_eq!(slot_of_path(r"base\gameplay\gui\fonts\foo.fnt"), None);
    }

    #[test]
    fn tweak_item_types() {
        let src = "Items.MyCoat:\n  $base: Items.GenericOuterChestClothing\nItems.MyBoots:\n  $base: Items.GenericFootClothing\n";
        assert_eq!(slots_of_tweak(src), BTreeSet::from(["outer", "feet"]));
        assert!(slots_of_tweak("Items.Foo:\n  $base: Items.Preset_Lexington\n").is_empty());
    }

    #[test]
    fn mod_files_and_listings_combine() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("a.yaml");
        std::fs::write(&yaml, "Items.Hood:\n  $base: Items.GenericHeadClothing\n  appearanceResourceName: x\n").unwrap();
        let xl = dir.path().join("a.xl");
        std::fs::write(&xl, "factories:\n  - base\\characters\\garment\\mine\\l1_900_pants.ent\n").unwrap();
        let files = vec![("r6/tweaks/a.yaml".to_string(), yaml), ("archive/pc/mod/a.xl".to_string(), xl)];
        let listed = vec![r"base\characters\common\hair\hh_900_new\hh_900.mesh".to_string()];
        assert_eq!(slots_of_mod(&files, &listed), BTreeSet::from(["head", "legs", "hair"]));
    }

    /// Runs the real WolvenKit CLI when `CPMX_WOLVENKIT_CLI` is set (see
    /// `wolvenkit::tests::real_cli_reads_our_icon_archive`).
    #[test]
    fn real_cli_listing_gives_a_slot() {
        let Some(dir) = std::env::var_os("CPMX_WOLVENKIT_CLI") else {
            return;
        };
        let Some(dotnet) = crate::wolvenkit::find_dotnet() else {
            return;
        };
        let cli = crate::wolvenkit::Cli { dir: PathBuf::from(dir), dotnet, version: "test".into() };
        let img = crate::inkicon::Rgba { width: 64, height: 64, pixels: [200, 40, 40, 255].repeat(64 * 64) };
        let built = crate::inkicon::build(&img, "base\\characters\\common\\hair\\cpmx_test").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("hair.archive");
        std::fs::write(&p, &built.archive).unwrap();
        let listed = cli.list_archive(&p).unwrap();
        eprintln!("{listed:?}");
        assert_eq!(slots_of_mod(&[], &listed), BTreeSet::from(["hair"]));
    }

    #[test]
    fn listing_cache_round_trip_and_rejects_odd_keys() {
        let dir = tempfile::tempdir().unwrap();
        store_listing(dir.path(), "abc123", &["base\\a.mesh".into(), "base\\b.xbm".into()]).unwrap();
        assert_eq!(cached_listing(dir.path(), "abc123").unwrap().len(), 2);
        assert!(cached_listing(dir.path(), "../x").is_none());
        store_listing(dir.path(), "../x", &[]).unwrap();
        assert!(!dir.path().parent().unwrap().join("x.txt").exists());
    }
}
