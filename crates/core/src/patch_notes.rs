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

/// Where a mod's last-update date comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSource {
    /// The Nexus mod page's last update (read with "Read release dates").
    NexusPage,
    /// The newest release on the mod's source (GitHub).
    Release,
    /// Only the installed file's upload date is known.
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// A core framework's installed file predates a game patch.
    Framework,
    /// A patch the mod predates reworked the systems mods build on.
    Rework,
    /// A patch the mod predates added what the mod does.
    Feature,
    /// The installed file is older than the mod's last update.
    NewerRelease,
    /// The Nexus page is no longer published.
    Unlisted,
}

#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub kind: NoteKind,
    /// The game patch it's about, if any.
    pub patch: Option<String>,
    pub text: String,
    pub link: Option<String>,
}

/// One installed mod's last update against the game patches.
#[derive(Debug, Clone, Serialize)]
pub struct ModAge {
    pub mod_id: i64,
    pub mod_name: String,
    /// Unix seconds.
    pub last_update: i64,
    pub date_source: DateSource,
    /// `last_update` is an upper bound (file older than every listed release).
    pub date_before: bool,
    /// Game patches (up to the installed version) released after
    /// `last_update`, oldest first. Empty: updated after the newest one.
    pub patches_since: Vec<String>,
    /// Release date of the first of `patches_since`.
    pub first_patch_date: Option<String>,
    /// Official notes of the first of `patches_since`.
    pub link: Option<String>,
    pub severity: Severity,
    /// Core framework name.
    pub framework: Option<String>,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Dated mods, the longest without an update first; mods updated after
    /// the newest patch last.
    pub mods: Vec<ModAge>,
    /// The game version the check used; `None` when unknown, in which case
    /// every listed patch counts.
    pub game_version: Option<String>,
    pub newest_patch: String,
    /// When the patch list was last compared with the official notes.
    pub data_checked: String,
    /// Names of mods without a date (manual installs, files without a date).
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
    let mut out = Vec::new();
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
        // The mod's last update: from its source when read, else the
        // installed file's date.
        let (last_update, date_source, date_before) = match (src.and_then(|s| s.latest), file) {
            (Some(l), _) => (l, if m.source == "nexus" { DateSource::NexusPage } else { DateSource::Release }, false),
            (None, Some(f)) => (f.at, DateSource::File, f.before),
            (None, None) => {
                undated.push(m.name.clone());
                continue;
            }
        };
        let since: Vec<&(&Patch, i64)> = applied.iter().filter(|(_, at)| last_update < *at).collect();
        let framework = framework_name(m);
        let mut notes = Vec::new();
        let mut severity = Severity::Info;

        if let Some(status) = src.and_then(|s| s.status.as_deref()).filter(|s| *s != "published") {
            notes.push(Note {
                kind: NoteKind::Unlisted,
                patch: None,
                text: format!("Nexus page status: {}. No further updates expected there.", status.replace('_', " ")),
                link: m.nexus_mod_id.map(|id| crate::nexus_browse::mod_page_url(id, None)),
            });
        }
        let fmt_file = |f: FileDate| if f.before { format!("before {}", ymd(f.at)) } else { ymd(f.at) };
        // Installed file older than the mod's last update.
        // A week's margin: a Nexus page date also moves with page edits.
        if let Some(f) = file.filter(|f| date_source != DateSource::File && f.at + 7 * 86_400 < last_update) {
            notes.push(Note {
                kind: NoteKind::NewerRelease,
                patch: None,
                text: format!(
                    "Installed file: {}; {}.",
                    fmt_file(f),
                    if date_source == DateSource::NexusPage { "Nexus page updated since (check for a newer file)" } else { "a newer release exists" }
                ),
                link: None,
            });
        }
        if let Some(name) = framework {
            // The modding wiki: after a game update, every core framework
            // needs a release from after that update.
            if let Some(f) = file
                && let Some((p, at)) = applied.iter().rev().find(|(_, at)| f.at < *at)
                && (date_source == DateSource::File || last_update > *at)
            {
                let confirmed = date_source != DateSource::File;
                if confirmed {
                    severity = Severity::Warning;
                }
                notes.push(Note {
                    kind: NoteKind::Framework,
                    patch: Some(p.version.clone()),
                    text: format!(
                        "{name} file predates game {} ({}). Core frameworks need a release from after the game update; {}.",
                        p.version,
                        p.date,
                        if confirmed { "a newer release exists" } else { "newer release not confirmed" }
                    ),
                    link: Some("https://wiki.redmodding.org/cyberpunk-2077-modding/for-mod-users/user-guide-troubleshooting/users-troubleshooting-after-a-game-update".into()),
                });
            }
        } else {
            if let Some((p, _)) = since.iter().rev().find(|(p, _)| p.rework) {
                if has_code(&mf.files) {
                    severity = Severity::Warning;
                }
                notes.push(Note {
                    kind: NoteKind::Rework,
                    patch: Some(p.version.clone()),
                    text: format!("Game {} reworked: {}.", p.version, p.changes.join("; ").trim_end_matches('.')),
                    link: Some(p.link.clone()),
                });
            }
            let text = words(&format!("{} {}", m.name, src.and_then(|s| s.summary.as_deref()).unwrap_or_default()));
            for (p, _) in &since {
                for f in p.features.iter().filter(|f| matches_feature(f, m.nexus_mod_id, &text)) {
                    notes.push(Note {
                        kind: NoteKind::Feature,
                        patch: Some(p.version.clone()),
                        text: format!("Possibly covered by game {}: {}", p.version, f.text),
                        link: Some(p.link.clone()),
                    });
                }
            }
        }
        out.push(ModAge {
            mod_id: m.id,
            mod_name: m.name.clone(),
            last_update,
            date_source,
            date_before,
            patches_since: since.iter().map(|(p, _)| p.version.clone()).collect(),
            first_patch_date: since.first().map(|(p, _)| p.date.clone()),
            link: since.first().map(|(p, _)| p.link.clone()),
            severity,
            framework: framework.map(String::from),
            notes,
        });
    }
    // Longest without an update first; warnings before hints on the same date.
    out.sort_by(|a, b| {
        a.patches_since
            .is_empty()
            .cmp(&b.patches_since.is_empty())
            .then_with(|| a.last_update.cmp(&b.last_update))
            .then_with(|| a.severity.cmp(&b.severity))
            .then_with(|| a.mod_name.to_lowercase().cmp(&b.mod_name.to_lowercase()))
    });
    Report {
        mods: out,
        game_version,
        newest_patch: list.last().map(|p| p.version.clone()).unwrap_or_default(),
        data_checked: data().checked.clone(),
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
    fn ranks_mods_by_last_update() {
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

        // Old Hair: the page was updated after 2.1, the installed file not.
        dates.insert("nexus:200".to_string(), SourceDates { latest: Some(t("2024-02-01")), ..Default::default() });

        let r = check_with(&list, Some("2.31"), &facts, &dates, None);
        let get = |id: i64| r.mods.iter().find(|m| m.mod_id == id).unwrap();
        let kinds = |id: i64| get(id).notes.iter().map(|n| n.kind).collect::<Vec<_>>();
        let m1 = get(1);
        assert_eq!(m1.patches_since, vec!["2.0", "2.1", "2.3"]);
        assert_eq!((m1.date_source, m1.severity, m1.first_patch_date.as_deref()), (DateSource::File, Severity::Warning, Some("2023-09-21")));
        assert!(m1.notes[0].text.contains("perks; police"), "{}", m1.notes[0].text);
        // Last update (Nexus page) decides, not the old file.
        let m2 = get(2);
        assert_eq!((m2.date_source, m2.severity), (DateSource::NexusPage, Severity::Info));
        assert_eq!(m2.patches_since, vec!["2.3"]);
        assert_eq!(kinds(2), vec![NoteKind::NewerRelease]);
        assert_eq!(kinds(3), vec![NoteKind::Feature]);
        assert_eq!(get(3).notes[0].patch.as_deref(), Some("2.1"));
        // Updated after 2.1, so no metro note; still before 2.3.
        assert_eq!(get(4).patches_since, vec!["2.3"]);
        assert!(kinds(4).is_empty());
        let m5 = get(5);
        assert_eq!((m5.framework.as_deref(), m5.severity), (Some("RED4ext"), Severity::Warning));
        assert!(m5.patches_since.is_empty(), "page updated after 2.3");
        assert_eq!(kinds(5), vec![NoteKind::NewerRelease, NoteKind::Framework]);
        assert!(kinds(7).contains(&NoteKind::Unlisted));
        assert_eq!(r.undated, vec!["Loose".to_string()]);
        assert_eq!(r.mods.len(), 6);
        // Longest without an update first, current mods last.
        assert_eq!(r.mods[0].mod_id, 1);
        assert!(r.mods.last().unwrap().patches_since.is_empty());

        // On game 2.1, 2.3 doesn't count.
        let r = check_with(&list, Some("2.1"), &facts, &dates, None);
        assert!(r.mods.iter().all(|m| !m.patches_since.contains(&"2.3".to_string())));
        assert!(r.mods.iter().find(|m| m.mod_id == 5).unwrap().notes.iter().all(|n| n.kind != NoteKind::Framework));
        // RED4ext without a release since 2.3: nothing to update to.
        dates.insert("nexus:2380".to_string(), SourceDates { latest: Some(t("2025-01-01")), ..Default::default() });
        let r = check_with(&list, Some("2.31"), &facts, &dates, None);
        let m5 = r.mods.iter().find(|m| m.mod_id == 5).unwrap();
        assert_eq!((m5.severity, m5.patches_since.clone()), (Severity::Info, vec!["2.3".to_string()]));
        assert!(m5.notes.is_empty());
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
