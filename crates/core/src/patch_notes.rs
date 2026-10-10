//! Installed mods against the game's patch notes: which game updates came
//! out after a mod's file or its newest release, and which features those
//! updates added to the base game. The patch list lives in
//! `data/patch_notes.json` (taken from the official patch notes on
//! cyberpunk.net); this module only evaluates it. Everything is a hint: the
//! check never turns a mod off.
//!
//! Dates come from the installed file's name (Nexus puts the upload time in
//! every file name) and, after "Read release dates", from Nexus and GitHub,
//! kept in the `release_dates` table so the check runs offline afterwards.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::{Db, ModRow};
use crate::dependencies::FRAMEWORKS;
use crate::known_issues::Severity;
use crate::modpacks::mod_key;
use crate::nexus::{now_unix, parse_time};
use crate::nexus_browse::ReleaseInfo;
use crate::reshade::ymd;
use crate::sources::ReleaseDates;
use crate::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: String,
    /// What the patch added, one sentence.
    pub text: String,
    /// Words or phrases (lowercase) that mark a mod as doing the same, matched
    /// as whole words in its name and Nexus summary.
    #[serde(default)]
    pub words: Vec<String>,
    /// Nexus mods known to provide this feature.
    #[serde(default)]
    pub nexus_mods: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    /// As the game's exe reports it ("2.0", "2.13", "2.3").
    pub version: String,
    /// PC release date, `YYYY-MM-DD`.
    pub date: String,
    pub title: String,
    /// The official patch notes.
    pub link: String,
    /// Reworked game systems that mods built on: mods older than this
    /// patch are flagged even without a matching feature.
    #[serde(default)]
    pub rework: bool,
    /// Reworked systems, for the explanation (only with `rework`).
    #[serde(default)]
    pub changes: Vec<String>,
    #[serde(default)]
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, Deserialize)]
struct Data {
    /// When the list was last compared with the official notes.
    checked: String,
    patches: Vec<Patch>,
}

fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/patch_notes.json")).expect("data/patch_notes.json is valid"))
}

/// Every listed patch, oldest first.
pub fn patches() -> &'static [Patch] {
    &data().patches
}

fn day(date: &str) -> Option<i64> {
    parse_time(&format!("{date}T00:00:00Z"))
}

/// Compare patch versions the way the game numbers them: the part after the
/// dot is a decimal fraction, so 2.13 < 2.2 < 2.21 < 2.3 < 2.31.
pub fn cmp_version(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    fn split(v: &str) -> Option<(u32, String)> {
        let v = v.trim().trim_start_matches(['v', 'V']);
        let (major, minor) = v.split_once('.').unwrap_or((v, ""));
        // "2.3.1" style: keep the digits only.
        let minor: String = minor.chars().filter(char::is_ascii_digit).collect();
        let minor = minor.trim_end_matches('0').to_string();
        Some((major.parse().ok()?, minor))
    }
    let (am, an) = split(a)?;
    let (bm, bn) = split(b)?;
    Some(am.cmp(&bm).then_with(|| an.cmp(&bn)))
}

/// The installed file's upload time from its Nexus file name
/// (`Name-<mod id>-<version>-<unix time>.zip`).
pub fn nexus_upload_time(archive_name: &str, nexus_mod_id: i64) -> Option<i64> {
    let stem = archive_name.rsplit_once('.').map_or(archive_name, |(s, _)| s);
    let parts: Vec<&str> = stem.split('-').collect();
    let last = *parts.last()?;
    let id = nexus_mod_id.to_string();
    if parts.len() < 3 || last.len() != 10 || !last.bytes().all(|b| b.is_ascii_digit()) || !parts[..parts.len() - 1].contains(&id.as_str()) {
        return None;
    }
    let t: i64 = last.parse().ok()?;
    // Nexus opened the game's section in December 2020.
    (1_606_000_000..=now_unix() + 86_400).contains(&t).then_some(t)
}

/// What Nexus or GitHub reported about one source mod.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceDates {
    /// Newest release (GitHub) or last update of the mod page (Nexus).
    pub latest: Option<i64>,
    pub summary: Option<String>,
    /// Nexus page status.
    pub status: Option<String>,
    /// File id (as stored in `mods`) -> its publication date.
    #[serde(default)]
    pub files: BTreeMap<String, FileDate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct FileDate {
    pub at: i64,
    /// Upper bound (see [`ReleaseDates::file_before`]).
    #[serde(default)]
    pub before: bool,
}

fn file_key(m: &ModRow) -> String {
    m.nexus_file_id.map(|f| f.to_string()).or(m.source_file.clone()).or(m.version.clone()).unwrap_or_default()
}

/// Stored dates for every mod key that has them, and when they were read.
pub fn saved(db: &Db) -> Result<(HashMap<String, SourceDates>, Option<i64>)> {
    let mut st = db.conn.prepare("SELECT mod_key, info, fetched_at FROM release_dates")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
    let mut out = HashMap::new();
    let mut newest = None;
    for r in rows {
        let (k, info, at) = r?;
        if let Ok(d) = serde_json::from_str(&info) {
            out.insert(k, d);
            newest = newest.max(Some(at));
        }
    }
    Ok((out, newest))
}

fn store(db: &Db, key: &str, d: &SourceDates, now: i64) -> Result<()> {
    db.conn.execute(
        "INSERT INTO release_dates (mod_key, info, fetched_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(mod_key) DO UPDATE SET info = excluded.info, fetched_at = excluded.fetched_at",
        params![key, serde_json::to_string(d)?, now],
    )?;
    Ok(())
}

/// Release dates read from the sources, to keep with [`save_fetched`].
#[derive(Debug, Default)]
pub struct Fetched {
    pub nexus: Vec<ReleaseInfo>,
    /// Nexus file uploads looked up for files whose name has no date:
    /// (mod id, file id, unix time).
    pub nexus_files: Vec<(i64, i64, i64)>,
    /// mod key and file key -> dates, for other sources.
    pub other: Vec<(String, String, ReleaseDates)>,
    pub errors: Vec<String>,
}

pub fn save_fetched(db: &Db, mods: &[ModRow], f: &Fetched) -> Result<()> {
    let (mut all, _) = saved(db)?;
    let now = now_unix();
    let mut touched: Vec<String> = Vec::new();
    for i in &f.nexus {
        let key = format!("nexus:{}", i.mod_id);
        let e = all.entry(key.clone()).or_default();
        e.latest = i.updated;
        e.summary = i.summary.clone();
        e.status = i.status.clone();
        touched.push(key);
    }
    for (mod_id, file_id, at) in &f.nexus_files {
        let key = format!("nexus:{mod_id}");
        all.entry(key.clone()).or_default().files.insert(file_id.to_string(), FileDate { at: *at, before: false });
        touched.push(key);
    }
    for (key, file, d) in &f.other {
        let e = all.entry(key.clone()).or_default();
        e.latest = d.latest;
        if let Some(at) = d.file {
            e.files.insert(file.clone(), FileDate { at, before: d.file_before });
        }
        touched.push(key.clone());
    }
    // Only keys of installed mods are kept.
    let keep: Vec<String> = mods.iter().map(mod_key).collect();
    touched.sort();
    touched.dedup();
    for k in touched.iter().filter(|k| keep.contains(k)) {
        store(db, k, &all[k], now)?;
    }
    Ok(())
}

/// Read release dates for `mods` from Nexus (one request per 50 mods, plus
/// a file list for files whose name carries no date) and the other sources.
/// Network only, so the caller doesn't hold the database.
pub fn fetch(mods: &[ModRow], nexus: Option<&crate::nexus::Client>, sources: &crate::sources::Registry) -> Fetched {
    let mut f = Fetched::default();
    let nexus_ids: Vec<i64> = mods.iter().filter(|m| m.source == "nexus").filter_map(|m| m.nexus_mod_id).collect();
    if let Some(client) = nexus {
        if !nexus_ids.is_empty() {
            match client.release_info(&nexus_ids) {
                Ok(v) => f.nexus = v,
                Err(e) => f.errors.push(format!("Nexus: {e}")),
            }
        }
        for m in mods.iter().filter(|m| m.source == "nexus") {
            let (Some(mid), Some(fid)) = (m.nexus_mod_id, m.nexus_file_id) else { continue };
            if nexus_upload_time(&m.archive_name, mid).is_some() {
                continue;
            }
            match client.files_with_updates(mid) {
                Ok((files, _)) => {
                    if let Some(at) = files.iter().find(|x| x.file_id == fid).and_then(|x| x.uploaded_timestamp) {
                        f.nexus_files.push((mid, fid, at));
                    }
                }
                Err(e) => f.errors.push(format!("{}: {e}", m.name)),
            }
        }
    } else if !nexus_ids.is_empty() {
        f.errors.push("Nexus: no API key; Nexus dates not read".into());
    }
    for m in mods.iter().filter(|m| m.source != "nexus" && m.source != "manual") {
        let (Ok(src), Some(source_ref)) = (sources.get(&m.source), m.source_ref.clone()) else { continue };
        let installed = crate::sources::InstalledRef {
            source_ref,
            file_id: m.source_file.clone(),
            file_name: Some(m.archive_name.clone()),
            version: m.version.clone(),
        };
        match src.release_dates(&installed) {
            Ok(Some(d)) => f.other.push((mod_key(m), file_key(m), d)),
            Ok(None) => {}
            Err(e) => f.errors.push(format!("{}: {e}", m.name)),
        }
    }
    f
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// A core framework older than a game update, with a newer release out.
    Framework,
    /// Released before a patch that reworked the systems mods build on.
    Rework,
    /// The base game added what the mod does.
    Feature,
    /// The Nexus page is no longer published.
    Unlisted,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub mod_id: i64,
    pub mod_name: String,
    pub kind: FindingKind,
    pub severity: Severity,
    /// The game patch it's about (none for `Unlisted`).
    pub patch: Option<String>,
    pub title: String,
    pub detail: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// The game version the check used; `None` when unknown, in which case
    /// every listed patch counts.
    pub game_version: Option<String>,
    pub newest_patch: String,
    /// When the patch list was last compared with the official notes.
    pub data_checked: String,
    /// Mods with a date to compare.
    pub dated: usize,
    /// Names of mods without one (manual installs, files without a date).
    pub undated: Vec<String>,
    /// When release dates were last read from the sources.
    pub dates_read_at: Option<i64>,
    /// Mods whose dates couldn't be read, with the reason (from "Read
    /// release dates" only).
    pub errors: Vec<String>,
}

/// One installed mod as the check sees it.
pub struct ModFacts<'a> {
    pub row: &'a ModRow,
    /// Game-relative paths, lowercase.
    pub files: Vec<String>,
}

/// Files that run code or change game records (scripts, CET, plugins,
/// TweakXL): the kind of mod a system rework breaks.
fn has_code(files: &[String]) -> bool {
    files.iter().any(|f| {
        [".reds", ".lua", ".dll", ".asi", ".yaml", ".yml", ".tweak", ".xl"].iter().any(|e| f.ends_with(e))
            || f.starts_with("r6/tweaks/")
    })
}

fn framework_name(m: &ModRow) -> Option<&'static str> {
    FRAMEWORKS
        .iter()
        .find(|f| {
            (m.nexus_mod_id.is_some() && f.2 == m.nexus_mod_id)
                || (m.source != "nexus" && f.3.is_some_and(|r| m.source_ref.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(r))))
        })
        .map(|f| f.1)
}

/// Lowercase words of `s` joined by single spaces, padded, so a phrase can
/// be found as whole words with `contains(" phrase ")`.
fn words(s: &str) -> String {
    let w: Vec<String> = s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    format!(" {} ", w.join(" "))
}

fn matches_feature(f: &Feature, nexus_mod_id: Option<i64>, text: &str) -> bool {
    nexus_mod_id.is_some_and(|id| f.nexus_mods.contains(&id)) || f.words.iter().any(|w| text.contains(&words(w)))
}

pub fn check(game_version: Option<&str>, mods: &[ModFacts], dates: &HashMap<String, SourceDates>, dates_read_at: Option<i64>) -> Report {
    check_with(patches(), game_version, mods, dates, dates_read_at)
}

fn check_with(
    list: &[Patch],
    game_version: Option<&str>,
    mods: &[ModFacts],
    dates: &HashMap<String, SourceDates>,
    dates_read_at: Option<i64>,
) -> Report {
    // Only "2.31"-style product versions; anything else counts as unknown.
    let game_version = game_version
        .map(str::trim)
        .filter(|v| v.matches('.').count() == 1 && cmp_version(v, "1.0").is_some())
        .map(String::from);
    // Patches the installed game has.
    let applied: Vec<(&Patch, i64)> = list
        .iter()
        .filter(|p| game_version.as_deref().is_none_or(|g| cmp_version(&p.version, g).is_some_and(|o| o.is_le())))
        .filter_map(|p| Some((p, day(&p.date)?)))
        .collect();
    let mut findings = Vec::new();
    let mut dated = 0;
    let mut undated = Vec::new();
    for mf in mods {
        let m = mf.row;
        let src = dates.get(&mod_key(m));
        let file = m
            .nexus_mod_id
            .filter(|_| m.source == "nexus")
            .and_then(|id| nexus_upload_time(&m.archive_name, id))
            .map(|at| FileDate { at, before: false })
            .or_else(|| src.and_then(|s| s.files.get(&file_key(m)).copied()));
        let latest = src.and_then(|s| s.latest);
        if let Some(status) = src.and_then(|s| s.status.as_deref()).filter(|s| *s != "published") {
            findings.push(Finding {
                mod_id: m.id,
                mod_name: m.name.clone(),
                kind: FindingKind::Unlisted,
                severity: Severity::Info,
                patch: None,
                title: format!("Nexus page status: {}", status.replace('_', " ")),
                detail: "The mod page is no longer published on Nexus; no further updates expected there.".into(),
                link: m.nexus_mod_id.map(|id| crate::nexus_browse::mod_page_url(id, None)),
            });
        }
        // The date the mod's content is from: its newest release when known
        // (an author who never updated after a patch), else the file's.
        let reference = match (latest, file) {
            (Some(l), _) => Some(l),
            (None, Some(f)) => Some(f.at),
            _ => None,
        };
        let Some(reference) = reference else {
            undated.push(m.name.clone());
            continue;
        };
        dated += 1;
        let fmt_file = |f: FileDate| if f.before { format!("before {}", ymd(f.at)) } else { ymd(f.at) };

        if let Some(name) = framework_name(m) {
            // The modding wiki: after a game update, every core framework
            // needs a release from after that update.
            let Some(f) = file else { continue };
            if let Some((p, at)) = applied.iter().rev().find(|(_, at)| f.at < *at) {
                let newer = latest.map(|l| l > *at);
                if newer == Some(false) {
                    continue; // no release since the patch: nothing to update to
                }
                findings.push(Finding {
                    mod_id: m.id,
                    mod_name: m.name.clone(),
                    kind: FindingKind::Framework,
                    severity: if newer == Some(true) { Severity::Warning } else { Severity::Info },
                    patch: Some(p.version.clone()),
                    title: format!("{name} file predates game {}", p.version),
                    detail: format!(
                        "Installed file: {}. Game {} released {}. Core frameworks need a release from after the game update{}.",
                        fmt_file(f),
                        p.version,
                        p.date,
                        match newer {
                            Some(true) => format!("; newer release: {}", ymd(latest.unwrap_or_default())),
                            _ => "; newer release not confirmed (Read release dates)".into(),
                        }
                    ),
                    link: Some("https://wiki.redmodding.org/cyberpunk-2077-modding/for-mod-users/user-guide-troubleshooting/users-troubleshooting-after-a-game-update".into()),
                });
            }
            continue;
        }

        let based_on = match (latest, file) {
            (Some(l), _) => if m.source == "nexus" { format!("Nexus page last updated: {}", ymd(l)) } else { format!("Newest release: {}", ymd(l)) },
            (None, Some(f)) => format!("Installed file: {}", fmt_file(f)),
            _ => unreachable!(),
        };
        // Installed file older than the patch while a newer release exists.
        let update_note = |at: i64| match (file, latest) {
            (Some(f), Some(l)) if f.at < at && l >= at => Some(format!(" Installed file: {}; a newer release exists.", fmt_file(f))),
            _ => None,
        };

        if let Some((p, _)) = applied.iter().rev().find(|(p, at)| p.rework && reference < *at) {
            findings.push(Finding {
                mod_id: m.id,
                mod_name: m.name.clone(),
                kind: FindingKind::Rework,
                severity: if has_code(&mf.files) { Severity::Warning } else { Severity::Info },
                patch: Some(p.version.clone()),
                title: format!("Released before game {}", p.version),
                detail: format!(
                    "{based_on}; game {} released {} and reworked: {}.",
                    p.version,
                    p.date,
                    p.changes.join("; ").trim_end_matches('.')
                ),
                link: Some(p.link.clone()),
            });
        } else if let Some((p, at)) = applied.iter().rev().find(|(p, at)| p.rework && file.is_some_and(|f| f.at < *at)) {
            // The author updated after the rework, the installed file didn't.
            findings.push(Finding {
                mod_id: m.id,
                mod_name: m.name.clone(),
                kind: FindingKind::Rework,
                severity: Severity::Info,
                patch: Some(p.version.clone()),
                title: format!("Installed file predates game {}", p.version),
                detail: format!("{}{}", format_args!("Game {} released {}.", p.version, p.date), update_note(*at).unwrap_or_default()),
                link: Some(p.link.clone()),
            });
        }

        let text = words(&format!("{} {}", m.name, src.and_then(|s| s.summary.as_deref()).unwrap_or_default()));
        for (p, at) in &applied {
            if reference >= *at {
                continue;
            }
            for f in p.features.iter().filter(|f| matches_feature(f, m.nexus_mod_id, &text)) {
                findings.push(Finding {
                    mod_id: m.id,
                    mod_name: m.name.clone(),
                    kind: FindingKind::Feature,
                    severity: Severity::Info,
                    patch: Some(p.version.clone()),
                    title: format!("Possibly covered by game {}", p.version),
                    detail: format!("Game {} ({}): {} {based_on}.", p.version, p.date, f.text),
                    link: Some(p.link.clone()),
                });
            }
        }
    }
    // Warnings first; per mod, the most specific finding first (it labels
    // the mod's row).
    let rank = |k: FindingKind| match k {
        FindingKind::Framework => 0,
        FindingKind::Feature => 1,
        FindingKind::Rework => 2,
        FindingKind::Unlisted => 3,
    };
    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.mod_name.to_lowercase().cmp(&b.mod_name.to_lowercase()))
            .then_with(|| rank(a.kind).cmp(&rank(b.kind)))
    });
    Report {
        findings,
        game_version,
        newest_patch: list.last().map(|p| p.version.clone()).unwrap_or_default(),
        data_checked: data().checked.clone(),
        dated,
        undated,
        dates_read_at,
        errors: Vec::new(),
    }
}

/// The check for a game's installed (enabled) mods, from the database only.
pub fn report(db: &Db, game_id: i64) -> Result<Report> {
    let game = db.game(game_id)?;
    let rows: Vec<ModRow> = db.mods(game_id)?.into_iter().filter(|m| m.enabled()).collect();
    let mut facts = Vec::new();
    for m in &rows {
        facts.push(ModFacts { row: m, files: db.mod_files(m.id)?.into_iter().map(|f| f.rel_path.to_lowercase()).collect() });
    }
    let (dates, read_at) = saved(db)?;
    let version = game.exe_product_version.clone();
    Ok(check(version.as_deref(), &facts, &dates, read_at))
}

/// When release dates were last read, if ever.
pub fn last_read(db: &Db) -> Result<Option<i64>> {
    Ok(db.conn.query_row("SELECT max(fetched_at) FROM release_dates", [], |r| r.get::<_, Option<i64>>(0)).optional()?.flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn data_file_parses_in_order() {
        let list = patches();
        assert!(list.len() >= 10);
        for w in list.windows(2) {
            assert_eq!(cmp_version(&w[0].version, &w[1].version), Some(Ordering::Less), "{} then {}", w[0].version, w[1].version);
            assert!(day(&w[0].date).unwrap() <= day(&w[1].date).unwrap(), "{} date", w[1].version);
        }
        for p in list {
            assert!(p.link.starts_with("https://"), "{} link", p.version);
            assert!(!p.rework || !p.changes.is_empty(), "{} rework without changes", p.version);
            for f in &p.features {
                assert!(!f.words.is_empty() || !f.nexus_mods.is_empty(), "{} matches nothing", f.id);
                assert!(f.words.iter().all(|w| w == &w.to_lowercase()), "{} words not lowercase", f.id);
            }
        }
    }

    #[test]
    fn versions_compare_as_patch_numbers() {
        assert_eq!(cmp_version("2.13", "2.2"), Some(Ordering::Less));
        assert_eq!(cmp_version("2.3", "2.31"), Some(Ordering::Less));
        assert_eq!(cmp_version("1.06", "1.1"), Some(Ordering::Less));
        assert_eq!(cmp_version("2.0", "2"), Some(Ordering::Equal));
        assert_eq!(cmp_version("2.10", "2.1"), Some(Ordering::Equal));
        assert_eq!(cmp_version("1.63", "2.0"), Some(Ordering::Less));
        assert_eq!(cmp_version("unknown", "2.0"), None);
    }

    #[test]
    fn reads_upload_time_from_nexus_names() {
        assert_eq!(nexus_upload_time("Release-107-0-2-0-1607860050.zip", 107), Some(1_607_860_050));
        assert_eq!(nexus_upload_time("Some Mod-1234-1-2-1700000000.7z", 1234), Some(1_700_000_000));
        // Mod id must appear; the stamp must be a plausible time.
        assert_eq!(nexus_upload_time("Some Mod-1234-1-2-1700000000.7z", 99), None);
        assert_eq!(nexus_upload_time("mod-1234-1-2-1234567890.zip", 1234), None);
        assert_eq!(nexus_upload_time("cet_1.35.0.zip", 107), None);
    }

    fn patch(v: &str, date: &str, rework: bool, features: Vec<Feature>) -> Patch {
        Patch {
            version: v.into(),
            date: date.into(),
            title: format!("Update {v}"),
            link: format!("https://example.invalid/{v}"),
            rework,
            changes: if rework { vec!["perks".into(), "police".into()] } else { vec![] },
            features,
        }
    }

    fn row(id: i64, name: &str, source: &str, nexus: Option<i64>, archive: &str) -> ModRow {
        ModRow {
            id,
            game_id: 1,
            name: name.into(),
            version: Some("1.0".into()),
            source: source.into(),
            nexus_mod_id: nexus,
            nexus_file_id: nexus.map(|n| n * 10),
            archive_name: archive.into(),
            archive_sha256: String::new(),
            archive_md5: String::new(),
            game_build_id: None,
            game_version: None,
            status: "installed".into(),
            installed_at: String::new(),
            file_count: 1,
            category: None,
            source_ref: None,
            source_file: None,
        }
    }

    fn t(date: &str) -> i64 {
        day(date).unwrap()
    }

    #[test]
    fn flags_old_mods_frameworks_and_covered_features() {
        let list = vec![
            patch("2.0", "2023-09-21", true, vec![]),
            patch(
                "2.1",
                "2023-12-05",
                false,
                vec![Feature { id: "metro".into(), text: "NCART metro rides.".into(), words: vec!["metro".into(), "ncart".into()], nexus_mods: vec![555] }],
            ),
            patch("2.3", "2025-07-17", false, vec![]),
        ];
        let stamp = |d: &str| t(d) + 3600;
        let rows = [
            // Script mod, last released before 2.0.
            row(1, "Old Perks", "nexus", Some(100), &format!("Old Perks-100-1-0-{}.zip", stamp("2023-05-01"))),
            // Texture-only mod before 2.0: info only.
            row(2, "Old Hair", "nexus", Some(200), &format!("Old Hair-200-1-{}.zip", stamp("2023-05-01"))),
            // Metro mod from before 2.1 found by its summary.
            row(3, "Train Rides", "nexus", Some(300), &format!("Train Rides-300-1-{}.zip", stamp("2023-10-01"))),
            // Known metro mod, updated after 2.1: not flagged.
            row(4, "City Metro", "nexus", Some(555), &format!("City Metro-555-2-{}.zip", stamp("2024-01-01"))),
            // RED4ext file from before 2.3 with a newer release.
            row(5, "RED4ext", "nexus", Some(2380), &format!("RED4ext-2380-1-{}.zip", stamp("2025-01-01"))),
            // Manual: no date.
            row(6, "Loose", "manual", None, "loose.zip"),
            // Removed from Nexus, recent.
            row(7, "Gone", "nexus", Some(700), &format!("Gone-700-1-{}.zip", stamp("2025-08-01"))),
        ];
        let files: [Vec<String>; 7] = [
            vec!["r6/scripts/perks.reds".into()],
            vec!["archive/pc/mod/hair.archive".into()],
            vec!["archive/pc/mod/train.archive".into()],
            vec!["archive/pc/mod/metro.archive".into()],
            vec!["red4ext/red4ext.dll".into()],
            vec!["archive/pc/mod/loose.archive".into()],
            vec!["archive/pc/mod/gone.archive".into()],
        ];
        let facts: Vec<ModFacts> = rows.iter().zip(files).map(|(row, files)| ModFacts { row, files }).collect();
        let mut dates = HashMap::new();
        dates.insert("nexus:300".to_string(), SourceDates { summary: Some("Ride the metro between districts".into()), ..Default::default() });
        dates.insert("nexus:2380".to_string(), SourceDates { latest: Some(t("2025-08-01")), ..Default::default() });
        dates.insert("nexus:700".to_string(), SourceDates { latest: Some(t("2025-08-01")), status: Some("removed".into()), ..Default::default() });

        let r = check_with(&list, Some("2.31"), &facts, &dates, None);
        let by = |id: i64| r.findings.iter().filter(|f| f.mod_id == id).collect::<Vec<_>>();
        let f1 = by(1);
        assert_eq!(f1.len(), 1);
        assert_eq!((f1[0].kind, f1[0].severity, f1[0].patch.as_deref()), (FindingKind::Rework, Severity::Warning, Some("2.0")));
        assert!(f1[0].detail.contains("perks; police"), "{}", f1[0].detail);
        assert_eq!(by(2)[0].severity, Severity::Info);
        let f3 = by(3);
        assert_eq!(f3.len(), 1);
        assert_eq!((f3[0].kind, f3[0].patch.as_deref()), (FindingKind::Feature, Some("2.1")));
        assert!(by(4).is_empty());
        let f5 = by(5);
        assert_eq!((f5[0].kind, f5[0].severity, f5[0].patch.as_deref()), (FindingKind::Framework, Severity::Warning, Some("2.3")));
        assert_eq!(by(7)[0].kind, FindingKind::Unlisted);
        assert_eq!(r.undated, vec!["Loose".to_string()]);
        assert_eq!(r.dated, 6);
        // Warnings first.
        assert_eq!(r.findings[0].severity, Severity::Warning);

        // On game 2.1, the 2.3 framework check doesn't apply.
        let r = check_with(&list, Some("2.1"), &facts, &dates, None);
        assert!(r.findings.iter().all(|f| f.mod_id != 5));
        // RED4ext without a newer release: nothing to update to.
        dates.insert("nexus:2380".to_string(), SourceDates { latest: Some(t("2025-01-01")), ..Default::default() });
        let r = check_with(&list, Some("2.31"), &facts, &dates, None);
        assert!(r.findings.iter().all(|f| f.mod_id != 5));
    }

    #[test]
    fn stores_and_reads_dates() {
        let db = Db::open_in_memory().unwrap();
        let mut m = row(1, "A", "github", None, "a.zip");
        m.source_ref = Some("o/r".into());
        m.source_file = Some("77".into());
        let f = Fetched {
            nexus: vec![],
            nexus_files: vec![],
            other: vec![("github:o/r".into(), "77".into(), ReleaseDates { file: Some(5), file_before: true, latest: Some(9) })],
            errors: vec![],
        };
        save_fetched(&db, std::slice::from_ref(&m), &f).unwrap();
        let (saved, at) = saved(&db).unwrap();
        assert!(at.is_some());
        let d = &saved["github:o/r"];
        assert_eq!(d.latest, Some(9));
        assert_eq!(d.files["77"], FileDate { at: 5, before: true });
        assert_eq!(last_read(&db).unwrap(), at);
    }
}
