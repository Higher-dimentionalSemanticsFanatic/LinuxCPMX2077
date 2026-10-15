//! The predefined tag set: groups of finer tags than Nexus categories
//! (Visuals › Textures, Lighting, …) that the user switches on per group,
//! and tag suggestions from what a mod's name, files, TweakXL records and
//! redscript targets show. The set and its rules live in
//! `data/tag_set.json`; switched-on tags are ordinary tags in the `tags`
//! table with their group in `tags.grp`. Suggestions are only offered.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::analysis::Kind;
use crate::db::{Db, ModRow};
use crate::modpacks::{self, mod_key};
use crate::{Error, Result};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub color: String,
    pub tags: Vec<SetTag>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetTag {
    pub name: String,
    pub about: String,
    /// Words in the mod name or Nexus category; 4 characters or fewer match
    /// whole words, longer ones the start of a word.
    #[serde(default)]
    words: Vec<String>,
    /// Nexus categories (any case).
    #[serde(default)]
    categories: Vec<String>,
    /// Game-relative globs: `*` within a folder, `**` across folders.
    #[serde(default)]
    paths: Vec<String>,
    /// TweakXL record prefixes, or substrings when they start with `*`.
    #[serde(default)]
    tweak: Vec<String>,
    /// Substrings of redscript class names.
    #[serde(default)]
    reds: Vec<String>,
    /// `mods.source` values.
    #[serde(default)]
    sources: Vec<String>,
    /// Mods from these sources never get the tag suggested.
    #[serde(default)]
    not_sources: Vec<String>,
    /// GitHub `owner/repo`s.
    #[serde(default)]
    repos: Vec<String>,
}

#[derive(Deserialize)]
struct Data {
    groups: Vec<Group>,
}

pub fn groups() -> &'static [Group] {
    static DATA: OnceLock<Data> = OnceLock::new();
    &DATA.get_or_init(|| serde_json::from_str(include_str!("../data/tag_set.json")).expect("data/tag_set.json is valid")).groups
}

fn group(id: &str) -> Result<&'static Group> {
    groups().iter().find(|g| g.id == id).ok_or_else(|| Error::Other(format!("no tag group called {id}")))
}

/// A group as the tag settings show it.
#[derive(Debug, Clone, Serialize)]
pub struct GroupState {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Any of its tags is in the tag list.
    pub enabled: bool,
    pub tags: Vec<TagState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagState {
    pub name: String,
    pub about: String,
}

pub fn group_states(db: &Db) -> Result<Vec<GroupState>> {
    let mut st = db.conn.prepare("SELECT DISTINCT grp FROM tags WHERE grp IS NOT NULL")?;
    let on: HashSet<String> = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(groups()
        .iter()
        .map(|g| GroupState {
            id: g.id.clone(),
            name: g.name.clone(),
            color: g.color.clone(),
            enabled: on.contains(&g.id),
            tags: g.tags.iter().map(|t| TagState { name: t.name.clone(), about: t.about.clone() }).collect(),
        })
        .collect())
}

/// Add a group's tags to the tag list. A tag the user already has under
/// the same name (any case) joins the group and keeps its color.
pub fn enable_group(db: &Db, id: &str) -> Result<()> {
    let g = group(id)?;
    let tx = db.conn.unchecked_transaction()?;
    for t in &g.tags {
        let name = modpacks::add_tag(db, &t.name, None)?;
        db.conn.execute("UPDATE tags SET grp = ?2, color = coalesce(color, ?3) WHERE name = ?1", params![name, g.id, g.color])?;
    }
    tx.commit()?;
    Ok(())
}

/// Mods that carry one of the group's tags.
pub fn group_usage(db: &Db, id: &str) -> Result<usize> {
    let n: i64 = db.conn.query_row(
        "SELECT count(DISTINCT m.mod_key) FROM mod_tags m JOIN tags t ON t.name = m.tag WHERE t.grp = ?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// Remove a group's tags from the tag list (and from every mod).
pub fn disable_group(db: &Db, id: &str) -> Result<()> {
    group(id)?;
    db.conn.execute("DELETE FROM tags WHERE grp = ?1", [id])?;
    Ok(())
}

pub const SUGGEST_SETTING: &str = "tag_suggestions";

/// Whether suggestions are shown (on unless switched off).
pub fn suggestions_on(db: &Db) -> Result<bool> {
    Ok(db.get_setting(SUGGEST_SETTING)?.as_deref() != Some("off"))
}

pub fn set_suggestions_on(db: &Db, on: bool) -> Result<()> {
    db.set_setting(SUGGEST_SETTING, if on { "on" } else { "off" })
}

/// One suggested tag and the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub tag: String,
    /// What matched, e.g. `name: "lighting"` or `TweakXL record Vehicle.v_sport1`.
    pub reason: String,
}

/// At most this many suggestions per mod, in tag-set order.
const MAX_PER_MOD: usize = 6;

/// What a mod shows, for matching.
struct Signals<'a> {
    /// ` word word … ` from the name and Nexus category, lowercase.
    words: String,
    category: Option<String>,
    source: &'a str,
    repo: Option<String>,
    files: Vec<String>,
    tweaks: Vec<String>,
    reds: Vec<String>,
}

fn words_of(s: &str) -> String {
    let w: Vec<String> = s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect();
    format!(" {} ", w.join(" "))
}

fn word_match(words: &str, w: &str) -> bool {
    let w = w.to_lowercase();
    let needle = if w.chars().count() <= 4 { format!(" {w} ") } else { format!(" {w}") };
    words.contains(&needle)
}

/// `*` matches within one path segment, `**` across segments. Lowercase in.
pub fn glob(pat: &str, s: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match p {
            [] => s.is_empty(),
            [b'*', b'*', rest @ ..] => (0..=s.len()).any(|i| go(rest, &s[i..])),
            [b'*', rest @ ..] => (0..=s.len()).take_while(|&i| i == 0 || s[i - 1] != b'/').any(|i| go(rest, &s[i..])),
            [c, rest @ ..] => s.first() == Some(c) && go(rest, &s[1..]),
        }
    }
    go(pat.as_bytes(), s.as_bytes())
}

fn tweak_match(pat: &str, key: &str) -> bool {
    let (p, k) = (pat.to_lowercase(), key.to_lowercase());
    match p.strip_prefix('*') {
        Some(sub) => k.contains(sub),
        None => k.starts_with(&p),
    }
}

/// First evidence for `t` in `s`, if any.
fn evidence(t: &SetTag, s: &Signals) -> Option<String> {
    if t.not_sources.iter().any(|x| x.eq_ignore_ascii_case(s.source)) {
        return None;
    }
    if let Some(x) = t.sources.iter().find(|x| x.eq_ignore_ascii_case(s.source)) {
        return Some(format!("source: {x}"));
    }
    if let Some(r) = s.repo.as_deref().filter(|r| t.repos.iter().any(|x| x.eq_ignore_ascii_case(r))) {
        return Some(format!("GitHub repo {r}"));
    }
    if let Some(c) = s.category.as_deref().filter(|c| t.categories.iter().any(|x| x.eq_ignore_ascii_case(c))) {
        return Some(format!("Nexus category {c}"));
    }
    if let Some(w) = t.words.iter().find(|w| word_match(&s.words, w)) {
        return Some(format!("name or category: \u{201c}{w}\u{201d}"));
    }
    for p in &t.paths {
        let p = p.to_lowercase();
        if let Some(f) = s.files.iter().find(|f| glob(&p, &f.to_lowercase())) {
            return Some(format!("file {f}"));
        }
    }
    for p in &t.tweak {
        if let Some(k) = s.tweaks.iter().find(|k| tweak_match(p, k)) {
            return Some(format!("TweakXL record {k}"));
        }
    }
    for p in &t.reds {
        let p = p.to_lowercase();
        if let Some(k) = s.reds.iter().find(|k| k.split('.').next().unwrap_or(k).to_lowercase().contains(&p)) {
            return Some(format!("redscript {k}"));
        }
    }
    None
}

/// Suggestions for `m` among `available` tags (lowercase names), skipping
/// the tags in `skip` (lowercase).
fn suggest_for(db: &Db, m: &ModRow, available: &HashMap<String, String>, skip: &HashSet<String>) -> Result<Vec<Suggestion>> {
    let candidates: Vec<&SetTag> = groups()
        .iter()
        .flat_map(|g| &g.tags)
        .filter(|t| available.contains_key(&t.name.to_lowercase()) && !skip.contains(&t.name.to_lowercase()))
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let (mut tweaks, mut reds) = (Vec::new(), Vec::new());
    for t in db.touches(m.id)? {
        match t.kind {
            Kind::TweakRecord => tweaks.push(t.key),
            Kind::RedsReplaceMethod | Kind::RedsWrapMethod | Kind::RedsAddMethod | Kind::RedsAddField => reds.push(t.key),
            _ => {}
        }
    }
    tweaks.sort();
    tweaks.dedup();
    reds.sort();
    reds.dedup();
    let signals = Signals {
        words: words_of(&format!("{} {}", m.name, m.category.as_deref().unwrap_or(""))),
        category: m.category.clone(),
        source: &m.source,
        repo: m.source_ref.clone(),
        files: db.mod_files(m.id)?.into_iter().map(|f| f.rel_path).collect(),
        tweaks,
        reds,
    };
    Ok(candidates
        .into_iter()
        .filter_map(|t| Some(Suggestion { tag: available[&t.name.to_lowercase()].clone(), reason: evidence(t, &signals)? }))
        .take(MAX_PER_MOD)
        .collect())
}

/// Installed mod id -> suggested tags, for the tags of the set that are in
/// the tag list. Tags a mod has or had dismissed are left out.
pub fn suggestions(db: &Db, game_id: i64) -> Result<HashMap<i64, Vec<Suggestion>>> {
    let available: HashMap<String, String> = modpacks::tags(db)?.into_iter().map(|t| (t.name.to_lowercase(), t.name)).collect();
    let mut have = modpacks::mod_tags(db)?;
    let mut dismissed: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut st = db.conn.prepare("SELECT mod_key, tag FROM tag_suggestions_dismissed")?;
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (k, t) = row?;
            dismissed.entry(k).or_default().push(t);
        }
    }
    let mut out = HashMap::new();
    for m in db.mods(game_id)? {
        let key = mod_key(&m);
        let skip: HashSet<String> =
            have.remove(&key).unwrap_or_default().into_iter().chain(dismissed.remove(&key).unwrap_or_default()).map(|t| t.to_lowercase()).collect();
        let s = suggest_for(db, &m, &available, &skip)?;
        if !s.is_empty() {
            out.insert(m.id, s);
        }
    }
    Ok(out)
}

/// Stop suggesting `tag` for the mod with `key`.
pub fn dismiss(db: &Db, key: &str, tag: &str) -> Result<()> {
    let tag = modpacks::clean_tag(tag)?;
    db.conn.execute("INSERT OR IGNORE INTO tag_suggestions_dismissed (mod_key, tag) VALUES (?1, ?2)", params![key, tag])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewMod;

    fn db_with_game() -> (Db, i64) {
        let db = Db::open_in_memory().unwrap();
        let game = crate::game::GameInstall {
            path: "/g".into(),
            store: crate::game::Store::Manual,
            proton_prefix: None,
            build_id: None,
            exe_file_version: None,
            exe_product_version: None,
            frameworks: vec![],
            launch_options: None,
            warnings: vec![],
        };
        let id = db.upsert_game(&game).unwrap();
        (db, id)
    }

    fn add_mod(db: &Db, game: i64, name: &str, category: Option<&str>, files: &[&str]) -> i64 {
        let id = db
            .insert_mod(
                &db.game(game).unwrap(),
                &NewMod { name: name.into(), source: "manual".into(), archive_name: format!("{name}.zip"), category: category.map(String::from), ..Default::default() },
            )
            .unwrap();
        for f in files {
            db.add_mod_file(&crate::db::ModFile { mod_id: id, rel_path: (*f).into(), staged_path: String::new(), sha256: String::new(), size: 0 })
                .unwrap();
        }
        id
    }

    #[test]
    fn data_is_valid() {
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        for g in groups() {
            assert!(ids.insert(g.id.clone()), "duplicate group {}", g.id);
            assert!(g.color.len() == 7 && g.color.starts_with('#'), "{}", g.color);
            for t in &g.tags {
                modpacks::clean_tag(&t.name).unwrap();
                assert!(names.insert(t.name.to_lowercase()), "duplicate tag {}", t.name);
            }
        }
    }

    #[test]
    fn globs() {
        assert!(glob("bin/x64/*.ini", "bin/x64/nova.ini"));
        assert!(!glob("bin/x64/*.ini", "bin/x64/plugins/cyber_engine_tweaks/mods/x/a.ini"));
        assert!(glob("bin/x64/reshade-shaders/**", "bin/x64/reshade-shaders/shaders/a.fx"));
        assert!(glob("mods/*/customsounds/**", "mods/gunsounds/customsounds/a.wav"));
    }

    #[test]
    fn words_match_whole_or_start() {
        let w = words_of("HDR Fix (Better-Lighting) 4K");
        assert!(word_match(&w, "4k") && word_match(&w, "lighting") && word_match(&w, "fix"));
        assert!(!word_match(&w, "hd"), "short words match whole words only");
        assert!(word_match(&words_of("Retextured roads"), "retex"));
    }

    #[test]
    fn enable_disable_and_suggest() {
        let (db, game) = db_with_game();
        // An own tag with a set name joins the group and keeps its color.
        modpacks::add_tag(&db, "lighting", Some("#123456")).unwrap();
        enable_group(&db, "visuals").unwrap();
        let tags = modpacks::tags(&db).unwrap();
        let light = tags.iter().find(|t| t.name == "lighting").unwrap();
        assert_eq!((light.color.as_deref(), light.group.as_deref()), (Some("#123456"), Some("visuals")));
        assert_eq!(tags.iter().find(|t| t.name == "Textures").unwrap().color.as_deref(), Some("#00f0ff"));
        assert!(group_states(&db).unwrap().iter().any(|g| g.id == "visuals" && g.enabled));

        let a = add_mod(&db, game, "Better Lighting 2.0", None, &[]);
        let b = add_mod(&db, game, "Some preset", None, &["bin/x64/Some.ini", "bin/x64/reshade-shaders/Shaders/a.fx"]);
        let s = suggestions(&db, game).unwrap();
        assert_eq!(s[&a], vec![Suggestion { tag: "lighting".into(), reason: "name or category: \u{201c}lighting\u{201d}".into() }]);
        let tags_b: Vec<&str> = s[&b].iter().map(|x| x.tag.as_str()).collect();
        assert_eq!(tags_b, ["Post-processing", "ReShade preset"]);

        // Having or dismissing a tag drops its suggestion.
        let mb = db.get_mod(b).unwrap();
        modpacks::set_mod_tags(&db, &mod_key(&mb), &["ReShade preset".into()]).unwrap();
        dismiss(&db, &mod_key(&mb), "Post-processing").unwrap();
        assert!(!suggestions(&db, game).unwrap().contains_key(&b));

        // Groups not switched on give no suggestions.
        let c = add_mod(&db, game, "Police Overhaul", None, &[]);
        assert!(!suggestions(&db, game).unwrap().contains_key(&c));

        assert_eq!(group_usage(&db, "visuals").unwrap(), 1);
        disable_group(&db, "visuals").unwrap();
        assert!(modpacks::tags(&db).unwrap().is_empty());
        assert!(modpacks::mod_tags(&db).unwrap().is_empty());
    }

    #[test]
    fn tweak_and_reds_evidence() {
        let t = groups().iter().flat_map(|g| &g.tags).find(|t| t.name == "Police & crime").unwrap();
        let s = Signals {
            words: words_of("x"),
            category: None,
            source: "nexus",
            repo: None,
            files: vec![],
            tweaks: vec![],
            reds: vec!["PreventionSystem.OnCrime".into()],
        };
        assert_eq!(evidence(t, &s).as_deref(), Some("redscript PreventionSystem.OnCrime"));
        let d = groups().iter().flat_map(|g| &g.tags).find(|t| t.name == "Driving & handling").unwrap();
        let s = Signals { tweaks: vec!["Vehicle.v_sport1_quadra_turbo_player_driveModel".into()], reds: vec![], ..s };
        assert!(evidence(d, &s).unwrap().starts_with("TweakXL record"));
    }
}
