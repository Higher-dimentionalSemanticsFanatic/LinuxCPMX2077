//! radioExt stations: list the stations in the game, and create or edit
//! stations from the user's own music files.
//!
//! radioExt (github.com/justarandomguyintheinternet/CP77_radioExt) is a CET
//! mod plus a RED4ext plugin. Each station is a folder in its `radios`
//! folder with a `metadata.json` (name, FM number, volume, icon, optional web
//! stream, play order) and the song files; the file names are the song names
//! in game.
//!
//! A station made here is installed as a tracked mod (`mods.source` =
//! [`SOURCE`], `source_ref` = its folder), so Disable, Uninstall, Verify,
//! profiles and the File map work on it like on any other mod. Saving an
//! edit installs the station again as a new version of that mod. Music files
//! are copied, never moved, and only formats radioExt plays are accepted,
//! checked by their content, not only their extension. Stations another mod
//! installed are listed read-only; folders no mod owns can be taken over by
//! saving them once.
//!
//! A station can get its own icon from a PNG: [`crate::inkicon`] turns it into
//! a texture and atlas in `archive/pc/mod/cpmx2077_radio_<slug>.archive`,
//! which is installed with the station (so it is enabled, disabled and
//! removed with it), and `customIcon` points at the atlas.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::activity::{self, Kind};
use crate::db::{GameRow, ModRow, NewMod};
use crate::install::{InstallOptions, InstallReport, Installer, Prepared, resolve_ci};
use crate::{Error, Result, inkicon};

/// radioExt's CET mod folder.
pub const MOD_DIR: &str = "bin/x64/plugins/cyber_engine_tweaks/mods/radioExt";
/// Where stations live.
pub const RADIOS: &str = "bin/x64/plugins/cyber_engine_tweaks/mods/radioExt/radios";
/// radioExt's RED4ext part; the CET part reports an error without it.
pub const PLUGIN: &str = "red4ext/plugins/RadioExt/RadioExt.dll";
/// Where radioExt is published.
pub const REPO: &str = "justarandomguyintheinternet/CP77_radioExt";
/// `mods.source` of a station made in the app.
pub const SOURCE: &str = "radioext";
pub const CATEGORY: &str = "Radio station";
/// The file radioExt reads a station's settings from.
pub const METADATA: &str = "metadata.json";
/// Song formats radioExt plays (its `radioManager.lua` extension list).
pub const AUDIO_EXTS: &[&str] = &["mp3", "mp2", "flac", "ogg", "wav", "wax", "wma", "opus", "aiff", "aif", "aifc"];
/// Formats only recent radioExt versions list; older ones skip these files.
pub const NEWER_EXTS: &[&str] = &["opus", "aiff", "aif", "aifc"];
/// The icon radioExt's template station uses.
pub const DEFAULT_ICON: &str = "UIIcon.alcohol_absynth";
/// Depot folder of the icons made here.
pub const ICON_DEPOT: &str = "cpmx2077\\radioext";
/// Where icon archives go.
pub const ICON_ARCHIVE_DIR: &str = "archive/pc/mod";
/// Bytes read from a song to find its tags.
const MAX_TAG_READ: u64 = 4 << 20;

fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// ASCII name of a station's icon files: the folder name reduced to
/// `[a-z0-9_]` plus a hash of the full folder name, so folders that differ
/// only in other characters still get their own icon.
pub fn icon_slug(folder: &str) -> String {
    let mut base = String::new();
    for c in folder.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('_') {
            base.push('_');
        }
    }
    let base: String = base.trim_matches('_').chars().take(40).collect();
    let hash = crate::inkicon::fnv1a64(folder.to_lowercase().as_bytes()) as u32;
    format!("{}_{hash:08x}", if base.is_empty() { "station" } else { base.trim_end_matches('_') })
}

/// Depot folder of a station's icon.
pub fn icon_depot_dir(folder: &str) -> String {
    format!("{ICON_DEPOT}\\{}", icon_slug(folder))
}

/// The atlas path `customIcon` names for a station's PNG icon.
pub fn icon_atlas_path(folder: &str) -> String {
    format!("{}\\icon.inkatlas", icon_depot_dir(folder))
}

/// Game-relative path of a station's icon archive.
pub fn icon_archive_rel(folder: &str) -> String {
    format!("{ICON_ARCHIVE_DIR}/cpmx2077_radio_{}.archive", icon_slug(folder))
}

/// The station's metadata points at an icon made here.
fn uses_png_icon(m: &Metadata, folder: &str) -> bool {
    m.custom_icon.use_custom && m.custom_icon.ink_atlas_path.eq_ignore_ascii_case(&icon_atlas_path(folder))
}

pub fn is_audio(name: &str) -> bool {
    AUDIO_EXTS.contains(&ext_of(name).as_str())
}

// ---- metadata.json -------------------------------------------------------------

/// A station's `metadata.json`. Fields radioExt adds in later versions are
/// kept as they are in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(default)]
    pub display_name: String,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub fm: f64,
    #[serde(default = "one", deserialize_with = "lenient_f64")]
    pub volume: f64,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub custom_icon: CustomIcon,
    #[serde(default)]
    pub stream_info: StreamInfo,
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomIcon {
    #[serde(default)]
    pub ink_atlas_path: String,
    #[serde(default)]
    pub ink_atlas_part: String,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub use_custom: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamInfo {
    #[serde(default, rename = "streamURL")]
    pub stream_url: String,
    #[serde(default, rename = "isStream", deserialize_with = "lenient_bool")]
    pub is_stream: bool,
}

fn one() -> f64 {
    1.0
}

// radioExt itself accepts "99.9" and "true" as strings, so reading does too.
fn lenient_f64<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<f64, D::Error> {
    match Value::deserialize(d)? {
        Value::Number(n) => Ok(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => s.trim().parse().map_err(serde::de::Error::custom),
        v => Err(serde::de::Error::custom(format!("expected a number, got {v}"))),
    }
}

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    match Value::deserialize(d)? {
        Value::Bool(b) => Ok(b),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Ok(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Ok(false),
        v => Err(serde::de::Error::custom(format!("expected true or false, got {v}"))),
    }
}

impl Metadata {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)? + "\n")
    }

    /// Why radioExt couldn't use these settings, if it couldn't.
    pub fn check(&self) -> Result<()> {
        let bad = |s: String| Err(Error::Other(s));
        if self.display_name.trim().is_empty() {
            return bad("station name is empty".into());
        }
        if !self.fm.is_finite() || self.fm <= 0.0 || self.fm >= 1000.0 {
            return bad(format!("FM {} is outside 0.1–999.9", self.fm));
        }
        if !self.volume.is_finite() || self.volume <= 0.0 || self.volume > 10.0 {
            return bad(format!("volume {} is outside 0.01–10", self.volume));
        }
        if self.stream_info.is_stream {
            let u = self.stream_info.stream_url.trim();
            if !(u.starts_with("http://") || u.starts_with("https://")) || u.contains(char::is_whitespace) {
                return bad("stream URL must start with http:// or https://".into());
            }
        }
        if self.custom_icon.use_custom {
            let p = self.custom_icon.ink_atlas_path.trim();
            if !p.to_ascii_lowercase().ends_with(".inkatlas") {
                return bad("custom icon path must name an .inkatlas file".into());
            }
            if self.custom_icon.ink_atlas_part.trim().is_empty() {
                return bad("custom icon part is empty".into());
            }
        } else if self.icon.trim().is_empty() {
            return bad("icon is empty; use a UIIcon record such as UIIcon.alcohol_absynth".into());
        }
        Ok(())
    }
}

// ---- song files ---------------------------------------------------------------

/// What a song file's first bytes say it is.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, m: &[u8]| head.get(i..i + m.len()) == Some(m);
    if at(0, b"ID3") || (head.len() > 1 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0) {
        Some("mpeg")
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        Some("wav")
    } else if at(0, b"OggS") {
        Some("ogg")
    } else if at(0, b"fLaC") {
        Some("flac")
    } else if at(0, &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11]) {
        Some("asf")
    } else if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        Some("aiff")
    } else if head.iter().find(|b| !b.is_ascii_whitespace()).is_some_and(|b| *b == b'<') {
        // A .wax is a Windows Media playlist (ASX, XML).
        Some("asx")
    } else {
        None
    }
}

/// The content kind each extension must have.
fn kind_of_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "mp3" | "mp2" => "mpeg",
        "wav" => "wav",
        "ogg" | "opus" => "ogg",
        "flac" => "flac",
        "wma" => "asf",
        "aiff" | "aif" | "aifc" => "aiff",
        "wax" => "asx",
        _ => return None,
    })
}

/// The extension a file of this content kind should have.
fn ext_for_kind(kind: &str) -> &'static str {
    match kind {
        "mpeg" => "mp3",
        "wav" => "wav",
        "ogg" => "ogg",
        "flac" => "flac",
        "asf" => "wma",
        "aiff" => "aiff",
        _ => "wax",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Tags {
    pub artist: Option<String>,
    pub title: Option<String>,
}

/// A song file the user picked, checked before it is added to a station.
#[derive(Debug, Clone, Serialize)]
pub struct SongInfo {
    pub path: String,
    pub size: u64,
    /// The file name in the station: "Artist - Title.ext" from the tags,
    /// else the original name, with the extension matching the content.
    pub name: String,
    /// The original name, with the extension matching the content.
    pub plain_name: String,
    pub tags: Tags,
    /// Why the file can't be used; `name` is then the original name.
    pub error: Option<String>,
    /// Plays only in recent radioExt versions.
    pub needs_recent: bool,
}

pub fn song_info(path: &Path) -> SongInfo {
    let original = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut info = SongInfo {
        path: path.to_string_lossy().into_owned(),
        size: 0,
        name: original.clone(),
        plain_name: original.clone(),
        tags: Tags::default(),
        error: None,
        needs_recent: false,
    };
    let checked = (|| -> Result<(String, String, Tags)> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(Error::Other("not a file".into()));
        }
        info.size = meta.len();
        let mut head = Vec::new();
        std::fs::File::open(path)?.take(MAX_TAG_READ).read_to_end(&mut head)?;
        let kind = check_content(&original, &head)?;
        let tags = read_tags(&head);
        let stem = Path::new(&original).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = ext_of(&original);
        let ext = if kind_of_ext(&ext) == Some(kind) { ext } else { ext_for_kind(kind).to_string() };
        let base = match (&tags.artist, &tags.title) {
            (Some(a), Some(t)) => format!("{a} - {t}"),
            (None, Some(t)) => t.clone(),
            _ => stem.clone(),
        };
        let plain = format!("{}.{ext}", clean_name(&stem, "song"));
        Ok((format!("{}.{ext}", clean_name(&base, "song")), plain, tags))
    })();
    match checked {
        Ok((name, plain, tags)) => {
            info.needs_recent = NEWER_EXTS.contains(&ext_of(&name).as_str());
            info.name = name;
            info.plain_name = plain;
            info.tags = tags;
        }
        Err(e) => info.error = Some(e.to_string()),
    }
    info
}

/// The content kind of an audio file, or why radioExt can't play it.
fn check_content(name: &str, head: &[u8]) -> Result<&'static str> {
    let ext = ext_of(name);
    let kind = sniff(head);
    match kind {
        Some(k) if k == "asx" && ext != "wax" => {
            Err(Error::Other(format!("{name}: text file, not audio")))
        }
        Some(k) => Ok(k),
        None if AUDIO_EXTS.contains(&ext.as_str()) => Err(Error::Other(format!("{name}: content is not {}", ext.to_uppercase()))),
        None => Err(Error::Other(format!(
            "{name}: format not played by radioExt (supported: {})",
            AUDIO_EXTS.join(", ")
        ))),
    }
}

/// Artist and title from ID3v2 (MP3), FLAC or Ogg (Vorbis/Opus) tags in the
/// first bytes of a file.
pub fn read_tags(head: &[u8]) -> Tags {
    let mut t = if head.starts_with(b"ID3") {
        id3v2(head)
    } else if head.starts_with(b"fLaC") {
        flac_tags(head)
    } else if head.starts_with(b"OggS") {
        ogg_tags(head)
    } else {
        Tags::default()
    };
    let tidy = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    t.artist = tidy(t.artist);
    t.title = tidy(t.title);
    t
}

fn syncsafe(b: &[u8]) -> usize {
    b.iter().take(4).fold(0, |n, x| (n << 7) | (*x as usize & 0x7F))
}

fn be(b: &[u8]) -> usize {
    b.iter().fold(0, |n, x| (n << 8) | *x as usize)
}

fn id3_text(data: &[u8]) -> Option<String> {
    let (enc, body) = data.split_first()?;
    let s = match enc {
        0 => body.iter().map(|&b| b as char).collect(),
        1 | 2 => {
            let (big, body) = match body {
                [0xFE, 0xFF, rest @ ..] => (true, rest),
                [0xFF, 0xFE, rest @ ..] => (false, rest),
                _ => (*enc == 2, body),
            };
            let units: Vec<u16> = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| if big { u16::from_be_bytes(*c) } else { u16::from_le_bytes(*c) })
                .collect();
            String::from_utf16_lossy(&units)
        }
        3 => String::from_utf8_lossy(body).into_owned(),
        _ => return None,
    };
    // Several values are separated by NUL; the first is the one shown.
    Some(s.split('\0').next().unwrap_or("").to_string())
}

fn id3v2(head: &[u8]) -> Tags {
    let mut t = Tags::default();
    if head.len() < 10 {
        return t;
    }
    let major = head[3];
    let flags = head[5];
    let end = (10 + syncsafe(&head[6..10])).min(head.len());
    let mut i = 10;
    // Unsynchronised tags would need decoding first; they're rare.
    if flags & 0x80 != 0 {
        return t;
    }
    if flags & 0x40 != 0 && head.len() >= i + 4 {
        i += if major >= 4 { syncsafe(&head[i..i + 4]) } else { be(&head[i..i + 4]) + 4 };
    }
    let (id_len, hdr) = if major == 2 { (3, 6) } else { (4, 10) };
    while i + hdr <= end {
        let id = &head[i..i + id_len];
        if id[0] == 0 {
            break;
        }
        let size = match major {
            2 => be(&head[i + 3..i + 6]),
            3 => be(&head[i + 4..i + 8]),
            _ => syncsafe(&head[i + 4..i + 8]),
        };
        let body = i + hdr;
        if size == 0 || body + size > end {
            break;
        }
        let data = &head[body..body + size];
        match id {
            b"TIT2" | b"TT2" => t.title = t.title.or_else(|| id3_text(data)),
            b"TPE1" | b"TP1" => t.artist = t.artist.or_else(|| id3_text(data)),
            _ => {}
        }
        i = body + size;
    }
    t
}

fn le32(b: &[u8], i: usize) -> Option<usize> {
    b.get(i..i + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize)
}

/// A Vorbis comment block (vendor string, then `KEY=value` entries).
fn vorbis_comments(b: &[u8]) -> Tags {
    let mut t = Tags::default();
    let Some(vendor) = le32(b, 0) else { return t };
    let mut i = 4 + vendor;
    let Some(count) = le32(b, i) else { return t };
    i += 4;
    for _ in 0..count.min(1000) {
        let Some(len) = le32(b, i) else { break };
        let Some(entry) = b.get(i + 4..i + 4 + len) else { break };
        i += 4 + len;
        let entry = String::from_utf8_lossy(entry);
        if let Some((k, v)) = entry.split_once('=') {
            if k.eq_ignore_ascii_case("TITLE") && t.title.is_none() {
                t.title = Some(v.to_string());
            } else if k.eq_ignore_ascii_case("ARTIST") && t.artist.is_none() {
                t.artist = Some(v.to_string());
            }
        }
    }
    t
}

fn flac_tags(head: &[u8]) -> Tags {
    let mut i = 4;
    while i + 4 <= head.len() {
        let (last, kind) = (head[i] & 0x80 != 0, head[i] & 0x7F);
        let len = be(&head[i + 1..i + 4]);
        let body = i + 4;
        if kind == 4 {
            return head.get(body..(body + len).min(head.len())).map(vorbis_comments).unwrap_or_default();
        }
        if last {
            break;
        }
        i = body + len;
    }
    Tags::default()
}

fn ogg_tags(head: &[u8]) -> Tags {
    // Join the packet data of the first pages, then look for the comment
    // header of Vorbis (\x03vorbis) or Opus (OpusTags).
    let mut data = Vec::new();
    let mut i = 0;
    while i + 27 <= head.len() && head[i..].starts_with(b"OggS") {
        let segs = head[i + 26] as usize;
        let Some(table) = head.get(i + 27..i + 27 + segs) else { break };
        let len: usize = table.iter().map(|&s| s as usize).sum();
        let start = i + 27 + segs;
        let Some(body) = head.get(start..start + len) else {
            data.extend_from_slice(&head[start.min(head.len())..]);
            break;
        };
        data.extend_from_slice(body);
        i = start + len;
    }
    for (magic, skip) in [(&b"\x03vorbis"[..], 7), (&b"OpusTags"[..], 8)] {
        if let Some(p) = data.windows(magic.len()).position(|w| w == magic) {
            return vorbis_comments(&data[p + skip..]);
        }
    }
    Tags::default()
}

/// A file or folder name that works on Windows and Linux.
pub fn clean_name(name: &str, fallback: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    let s = s.trim().trim_end_matches(['.', ' ']).trim();
    let s: String = s.chars().take(120).collect();
    let s = s.trim_end_matches(['.', ' ']).to_string();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    let stem = s.split('.').next().unwrap_or("");
    if s.is_empty() {
        fallback.to_string()
    } else if reserved.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        format!("_{s}")
    } else {
        s
    }
}

// ---- stations in the game -------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Song {
    pub file: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Station {
    pub folder: String,
    pub metadata: Option<Metadata>,
    /// Missing or unreadable `metadata.json`, with the reason.
    pub problem: Option<String>,
    pub songs: Vec<Song>,
    /// Files radioExt doesn't play (pictures, notes).
    pub other_files: Vec<String>,
    /// What radioExt would complain about in its CET console.
    pub warnings: Vec<String>,
    /// The mod the station's files belong to.
    pub mod_id: Option<i64>,
    pub mod_name: Option<String>,
    /// Made in the app (editable here).
    pub managed: bool,
    pub enabled: bool,
    /// Game-relative path of the icon archive made here from a PNG, when the
    /// station uses one.
    pub icon_archive: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// radioExt's CET part (`init.lua`) is in the game.
    pub installed: bool,
    /// radioExt's RED4ext part is in the game.
    pub plugin: bool,
    /// The mod radioExt was installed with.
    pub mod_name: Option<String>,
    pub stations: Vec<Station>,
    /// Icons the listed stations use, for suggestions.
    pub icons: Vec<String>,
}

/// `rel` below the folder `pfx` (ending in `/`), matched without regard to
/// ASCII case.
fn in_folder(rel: &str, pfx: &str) -> Option<String> {
    rel.get(..pfx.len()).filter(|h| h.eq_ignore_ascii_case(pfx)).map(|_| rel[pfx.len()..].to_string())
}

/// The app-made station mods of a game, newest last.
fn station_mods(inst: &Installer, game_id: i64) -> Result<Vec<ModRow>> {
    Ok(inst.db.mods(game_id)?.into_iter().filter(|m| m.source == SOURCE).collect())
}

fn folder_of(m: &ModRow) -> String {
    m.source_ref.clone().unwrap_or_default()
}

fn station_from(folder: String, files: Vec<(String, u64)>, metadata: Option<Result<Vec<u8>>>) -> Station {
    let (metadata, problem) = match metadata {
        None => (None, Some(format!("no {METADATA}; radioExt skips this folder"))),
        Some(Err(e)) => (None, Some(format!("{METADATA} not read: {e}"))),
        Some(Ok(b)) => match Metadata::parse(&b) {
            Ok(m) => (Some(m), None),
            Err(e) => (None, Some(format!("{METADATA} is not valid: {e}"))),
        },
    };
    let mut songs = Vec::new();
    let mut other_files = Vec::new();
    for (f, size) in files {
        if f.eq_ignore_ascii_case(METADATA) {
            continue;
        }
        if is_audio(&f) {
            songs.push(Song { file: f, size });
        } else {
            other_files.push(f);
        }
    }
    let mut warnings = Vec::new();
    if let Some(m) = &metadata {
        if songs.is_empty() && !m.stream_info.is_stream {
            warnings.push("no song files and not a stream: radioExt reports an error".into());
        }
        let have: HashSet<&str> = songs.iter().map(|s| s.file.as_str()).collect();
        for o in &m.order {
            if !have.contains(o.as_str()) {
                warnings.push(format!("play order names {o}, which is not in the folder"));
            }
        }
    }
    Station {
        folder,
        metadata,
        problem,
        songs,
        other_files,
        warnings,
        mod_id: None,
        mod_name: None,
        managed: false,
        enabled: true,
        icon_archive: None,
    }
}

pub fn status(inst: &Installer, game: &GameRow) -> Result<Status> {
    let db = inst.db;
    let game_dir = Path::new(&game.path);
    let init = format!("{MOD_DIR}/init.lua");
    let installed = resolve_ci(game_dir, &init).is_file();
    let plugin = resolve_ci(game_dir, PLUGIN).is_file();
    let mod_name = db.owners_of(game.id, &init, -1)?.first().map(|f| db.get_mod(f.mod_id)).transpose()?.map(|m| m.name);
    let managed: Vec<ModRow> = station_mods(inst, game.id)?;

    let mut stations = Vec::new();
    let radios = resolve_ci(game_dir, RADIOS);
    if let Ok(rd) = std::fs::read_dir(&radios) {
        for e in rd.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let folder = e.file_name().to_string_lossy().into_owned();
            let mut files = Vec::new();
            for f in std::fs::read_dir(e.path())?.flatten() {
                if f.file_type().is_ok_and(|t| t.is_file()) {
                    files.push((f.file_name().to_string_lossy().into_owned(), f.metadata().map(|m| m.len()).unwrap_or(0)));
                }
            }
            let meta_file = files.iter().find(|(f, _)| f.eq_ignore_ascii_case(METADATA)).map(|(f, _)| e.path().join(f));
            let mut s = station_from(folder.clone(), files, meta_file.map(|p| Ok(std::fs::read(p)?)));
            // Who owns it: the owner of metadata.json, else of any file.
            let mut owner = None;
            for name in std::iter::once(METADATA.to_string()).chain(s.songs.iter().map(|x| x.file.clone())) {
                if let Some(f) = db.owners_of(game.id, &format!("{RADIOS}/{folder}/{name}"), -1)?.first() {
                    owner = Some(db.get_mod(f.mod_id)?);
                    break;
                }
            }
            if let Some(m) = owner {
                s.managed = m.source == SOURCE && folder_of(&m).eq_ignore_ascii_case(&folder);
                s.mod_id = Some(m.id);
                s.mod_name = Some(m.name);
            }
            stations.push(s);
        }
    }
    // Disabled app-made stations aren't in the game; read their stored copy.
    for m in managed.iter().filter(|m| !m.enabled()) {
        let folder = folder_of(m);
        let pfx = format!("{RADIOS}/{folder}/");
        let stage = inst.staging_root.join(m.id.to_string());
        let mut files = Vec::new();
        let mut meta = None;
        for f in db.mod_files(m.id)? {
            let Some(name) = in_folder(&f.rel_path, &pfx) else {
                continue;
            };
            if name.eq_ignore_ascii_case(METADATA) {
                meta = Some(std::fs::read(stage.join(&f.staged_path)).map_err(Error::from));
            }
            files.push((name, f.size.max(0) as u64));
        }
        let mut s = station_from(folder, files, meta);
        s.managed = true;
        s.enabled = false;
        s.mod_id = Some(m.id);
        s.mod_name = Some(m.name.clone());
        stations.push(s);
    }
    for s in &mut stations {
        if s.metadata.as_ref().is_some_and(|m| uses_png_icon(m, &s.folder)) {
            s.icon_archive = Some(icon_archive_rel(&s.folder));
            if s.enabled && !resolve_ci(game_dir, &icon_archive_rel(&s.folder)).is_file() {
                s.warnings.push(format!("icon archive {} is missing; the station shows no icon", icon_archive_rel(&s.folder)));
            }
        }
    }
    stations.sort_by(|a, b| {
        let fm = |s: &Station| s.metadata.as_ref().map(|m| m.fm).unwrap_or(f64::MAX);
        fm(a).total_cmp(&fm(b)).then_with(|| a.folder.to_lowercase().cmp(&b.folder.to_lowercase()))
    });
    let mut icons: Vec<String> = stations
        .iter()
        .filter_map(|s| s.metadata.as_ref())
        .map(|m| m.icon.trim().to_string())
        .filter(|i| i.starts_with("UIIcon."))
        .chain(std::iter::once(DEFAULT_ICON.to_string()))
        .collect();
    icons.sort_by_key(|i| i.to_lowercase());
    icons.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(Status { installed, plugin, mod_name, stations, icons })
}

// ---- creating and editing ----------------------------------------------------------

/// One song of a station being saved, in play order.
#[derive(Debug, Clone, Deserialize)]
pub struct DraftSong {
    /// File name in the station (the song name in game).
    pub name: String,
    /// A song already in the station, by its current file name.
    #[serde(default)]
    pub keep: Option<String>,
    /// A file on disk to copy in.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Draft {
    /// The station's folder; `None` for a new station.
    #[serde(default)]
    pub folder: Option<String>,
    pub metadata: Metadata,
    pub songs: Vec<DraftSong>,
    /// Write the song list as radioExt's play order; otherwise shuffled.
    #[serde(default)]
    pub fixed_order: bool,
    /// A PNG to make the station's icon from.
    #[serde(default)]
    pub icon_png: Option<String>,
}

/// Where the current files of a station being edited come from.
enum Origin {
    New,
    /// An app-made station: its stored copy.
    Managed(Box<ModRow>),
    /// A folder no mod owns, moved aside while the station is installed.
    Untracked(PathBuf),
}

/// Where a station's icon archive comes from when saving.
enum IconSource {
    /// Made from a PNG just now.
    Bytes(Vec<u8>),
    /// The stored copy of the one made before.
    Stored(PathBuf),
}

fn folder_taken(inst: &Installer, game: &GameRow, name: &str) -> Result<bool> {
    let radios = resolve_ci(Path::new(&game.path), RADIOS);
    let on_disk = std::fs::read_dir(&radios)
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)))
        .unwrap_or(false);
    Ok(on_disk || station_mods(inst, game.id)?.iter().any(|m| folder_of(m).eq_ignore_ascii_case(name)))
}

/// A free folder name for a new station called `display_name`.
pub fn new_folder(inst: &Installer, game: &GameRow, display_name: &str) -> Result<String> {
    let base = clean_name(display_name, "station");
    let mut name = base.clone();
    let mut n = 2;
    while folder_taken(inst, game, &name)? {
        name = format!("{base} ({n})");
        n += 1;
    }
    Ok(name)
}

fn link_or_copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// Install `draft` as a new station, or as the new version of the station in
/// `draft.folder`. User files are copied; the stored copy of an app-made
/// station is reused without copying where the file system allows it.
pub fn save(inst: &Installer, game: &GameRow, draft: &Draft) -> Result<InstallReport> {
    let mut metadata = draft.metadata.clone();
    metadata.display_name = metadata.display_name.trim().to_string();
    metadata.icon = metadata.icon.trim().to_string();
    metadata.stream_info.stream_url = metadata.stream_info.stream_url.trim().to_string();
    metadata.custom_icon.ink_atlas_path = metadata.custom_icon.ink_atlas_path.trim().to_string();
    metadata.custom_icon.ink_atlas_part = metadata.custom_icon.ink_atlas_part.trim().to_string();
    metadata.check()?;
    if draft.songs.is_empty() && !metadata.stream_info.is_stream {
        return Err(Error::Other("add at least one song, or set a stream URL".into()));
    }
    let game_dir = Path::new(&game.path);

    // Where the station is now.
    let (folder, origin) = match &draft.folder {
        None => (new_folder(inst, game, &metadata.display_name)?, Origin::New),
        Some(folder) => {
            if let Some(m) = station_mods(inst, game.id)?.into_iter().find(|m| folder_of(m).eq_ignore_ascii_case(folder)) {
                (folder_of(&m), Origin::Managed(Box::new(m)))
            } else {
                let dir = resolve_ci(game_dir, &format!("{RADIOS}/{folder}"));
                if !dir.is_dir() || folder.contains(['/', '\\']) || folder.starts_with('.') {
                    return Err(Error::Other(format!("no station folder {folder}")));
                }
                let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                for f in std::fs::read_dir(&dir)?.flatten() {
                    let rel = format!("{RADIOS}/{folder}/{}", f.file_name().to_string_lossy());
                    if let Some(o) = inst.db.owners_of(game.id, &rel, -1)?.first() {
                        let m = inst.db.get_mod(o.mod_id)?;
                        return Err(Error::Conflict(format!(
                            "{folder} has files from {}; change it through that mod, or disable it and create a new station",
                            m.name
                        )));
                    }
                }
                (folder, Origin::Untracked(dir))
            }
        }
    };

    // The station's current files by name.
    let current: Vec<(String, PathBuf)> = match &origin {
        Origin::New => vec![],
        Origin::Managed(m) => {
            let pfx = format!("{RADIOS}/{folder}/");
            let stage = inst.staging_root.join(m.id.to_string());
            inst.db
                .mod_files(m.id)?
                .into_iter()
                .filter_map(|f| Some((in_folder(&f.rel_path, &pfx)?, stage.join(&f.staged_path))))
                .collect()
        }
        Origin::Untracked(dir) => std::fs::read_dir(dir)?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect(),
    };
    let find_current = |name: &str| current.iter().find(|(n, _)| n == name).map(|(_, p)| p.clone());

    // The icon archive: made from a new PNG, or the one made before.
    let icon_rel = icon_archive_rel(&folder);
    let icon: Option<IconSource> = if let Some(png) = draft.icon_png.as_deref().filter(|p| !p.trim().is_empty()) {
        let img = inkicon::load_png(Path::new(png)).map_err(|e| Error::Other(format!("icon: {e}")))?;
        let made = inkicon::build(&img, &icon_depot_dir(&folder))?;
        metadata.custom_icon = CustomIcon { ink_atlas_path: made.atlas_path, ink_atlas_part: made.part.into(), use_custom: true };
        Some(IconSource::Bytes(made.archive))
    } else if uses_png_icon(&metadata, &folder) {
        let stored = match &origin {
            Origin::Managed(m) => inst
                .db
                .mod_files(m.id)?
                .into_iter()
                .find(|f| f.rel_path.eq_ignore_ascii_case(&icon_rel))
                .map(|f| inst.staging_root.join(m.id.to_string()).join(f.staged_path)),
            _ => None,
        };
        Some(IconSource::Stored(
            stored.filter(|p| p.is_file()).ok_or_else(|| Error::Other("the station's icon archive is missing; choose the PNG again".into()))?,
        ))
    } else {
        None
    };

    // Check every song before touching anything.
    let mut names = HashSet::new();
    let mut plan: Vec<(String, PathBuf, bool)> = Vec::new(); // name, source, user file
    for s in &draft.songs {
        let name = s.name.trim();
        if name != clean_name(name, "") || name.starts_with('.') {
            return Err(Error::Other(format!("{name}: not a valid file name")));
        }
        if !is_audio(name) {
            return Err(Error::Other(format!("{name}: song names must end in one of .{}", AUDIO_EXTS.join(", ."))));
        }
        if name.eq_ignore_ascii_case(METADATA) || !names.insert(name.to_lowercase()) {
            return Err(Error::Other(format!("{name} is in the list twice")));
        }
        let (src, user) = match (&s.keep, &s.path) {
            (Some(k), None) => (find_current(k).ok_or_else(|| Error::Other(format!("{k} is no longer in the station")))?, false),
            (None, Some(p)) => (PathBuf::from(p), true),
            _ => return Err(Error::Other(format!("{name}: give either a station song or a file"))),
        };
        if user {
            let mut head = Vec::new();
            std::fs::File::open(&src)?.take(64).read_to_end(&mut head)?;
            let kind = check_content(&src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), &head)?;
            if kind_of_ext(&ext_of(name)) != Some(kind) {
                return Err(Error::Other(format!("{name}: content is {}, extension should be .{}", kind.to_uppercase(), ext_for_kind(kind))));
            }
        }
        plan.push((name.to_string(), src, user));
    }
    // Pictures and notes in the folder stay; songs left out are dropped.
    let keep_other: Vec<&(String, PathBuf)> =
        current.iter().filter(|(n, _)| !is_audio(n) && !n.eq_ignore_ascii_case(METADATA) && !names.contains(&n.to_lowercase())).collect();
    metadata.order = if draft.fixed_order { plan.iter().map(|(n, _, _)| n.clone()).collect() } else { vec![] };

    // Stage the station laid out like the game folder.
    std::fs::create_dir_all(&inst.staging_root)?;
    let dir = tempfile::Builder::new().prefix("incoming-").tempdir_in(&inst.staging_root)?.keep();
    let id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = format!("{RADIOS}/{folder}");
    let staged = (|| -> Result<Vec<String>> {
        let mut files = Vec::new();
        let mut put = |name: &str, src: &Path, copy: bool| -> Result<()> {
            let rel = format!("{base}/{name}");
            let dst = dir.join("files").join(&rel);
            if copy {
                std::fs::create_dir_all(dst.parent().unwrap_or(&dir))?;
                std::fs::copy(src, &dst)?;
            } else {
                link_or_copy(src, &dst)?;
            }
            files.push(rel);
            Ok(())
        };
        for (name, src, user) in &plan {
            // Stored copies may be hard-linked; anything in the game folder
            // or picked by the user is copied.
            put(name, src, *user || !matches!(origin, Origin::Managed(_)))?;
        }
        for (name, src) in &keep_other {
            put(name, src, !matches!(origin, Origin::Managed(_)))?;
        }
        if let Some(icon) = &icon {
            let dst = dir.join("files").join(&icon_rel);
            std::fs::create_dir_all(dst.parent().unwrap_or(&dir))?;
            match icon {
                IconSource::Bytes(b) => std::fs::write(&dst, b)?,
                IconSource::Stored(p) => link_or_copy(p, &dst)?,
            }
            files.push(icon_rel.clone());
        }
        let meta_rel = format!("{base}/{METADATA}");
        let meta_path = dir.join("files").join(&meta_rel);
        std::fs::create_dir_all(meta_path.parent().unwrap_or(&dir))?;
        std::fs::write(&meta_path, metadata.to_json()?)?;
        files.push(meta_rel);
        Ok(files)
    })();
    let files = match staged {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let p = Prepared {
        id,
        archive_name: format!("{folder} (radio station)"),
        archive_sha256: String::new(),
        archive_md5: String::new(),
        file_count: files.len(),
        fomod: None,
        dir,
        files,
    };
    let meta = NewMod {
        name: metadata.display_name.clone(),
        source: SOURCE.into(),
        category: Some(CATEGORY.into()),
        source_ref: Some(folder.clone()),
        ..Default::default()
    };
    let opts = InstallOptions { meta, overwrite: false, fomod_choices: None };
    let result = match &origin {
        Origin::New => inst.finish(game, &p, opts),
        Origin::Managed(old) => inst.finish_replacing(game, &p, opts, old.id),
        Origin::Untracked(dir) => {
            // Move the folder aside so the install doesn't count its files as
            // game files to back up; put it back if the install fails.
            let aside = dir.with_file_name(format!(".cpmx-saving-{folder}"));
            std::fs::rename(dir, &aside)?;
            match inst.finish(game, &p, opts) {
                Ok(r) => {
                    std::fs::remove_dir_all(&aside)?;
                    Ok(r)
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(dir);
                    std::fs::rename(&aside, dir)?;
                    Err(e)
                }
            }
        }
    };
    match result {
        Ok(r) => {
            activity::record_path(Kind::Info, format!("Saved radio station {}", metadata.display_name), &resolve_ci(game_dir, &base));
            Ok(r)
        }
        Err(e) => {
            inst.discard(&p);
            Err(e)
        }
    }
}

/// Depot path and bytes of each file in an icon archive.
pub type IconFiles = Vec<(String, Vec<u8>)>;

/// A station's icon archive in the game folder and the texture and atlas it
/// holds (read with [`inkicon::read_archive`]), for checking with WolvenKit.
pub fn icon_files(game: &GameRow, folder: &str) -> Result<(PathBuf, IconFiles)> {
    let rel = icon_archive_rel(folder);
    let path = resolve_ci(Path::new(&game.path), &rel);
    if !path.is_file() {
        return Err(Error::Other(format!("{rel} is not in the game folder")));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?.take(64 << 20).read_to_end(&mut bytes)?;
    let files = inkicon::read_archive(&bytes)?;
    let dir = icon_depot_dir(folder);
    let mut out = Vec::new();
    for name in ["icon.xbm", "icon.inkatlas"] {
        let p = format!("{dir}\\{name}");
        let h = inkicon::path_hash(&p);
        let (_, b) = files.iter().find(|(x, _)| *x == h).ok_or_else(|| Error::Other(format!("{rel} has no {p}")))?;
        out.push((p, b.clone()));
    }
    Ok((path, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reshade::tests::{installer, setup_game};

    const MP3: &[u8] = b"ID3\x03\x00\x00\x00\x00\x00\x23TIT2\x00\x00\x00\x06\x00\x00\x00NightTPE1\x00\x00\x00\x05\x00\x00\x00Ada\x00\x00\xFF\xFB\x90\x00";

    fn flac() -> Vec<u8> {
        let mut c = Vec::new();
        c.extend(3u32.to_le_bytes());
        c.extend(b"ref");
        c.extend(2u32.to_le_bytes());
        for e in ["ARTIST=Samurai", "title=Chippin' In"] {
            c.extend((e.len() as u32).to_le_bytes());
            c.extend(e.as_bytes());
        }
        let mut f = b"fLaC".to_vec();
        f.extend([0x00, 0, 0, 2, 0xAA, 0xBB]); // STREAMINFO (short, fine here)
        f.push(0x84);
        f.extend(&(c.len() as u32).to_be_bytes()[1..]);
        f.extend(c);
        f
    }

    #[test]
    fn reads_tags_and_checks_content() {
        let t = read_tags(MP3);
        assert_eq!((t.artist.as_deref(), t.title.as_deref()), (Some("Ada"), Some("Night")));
        let t = read_tags(&flac());
        assert_eq!((t.artist.as_deref(), t.title.as_deref()), (Some("Samurai"), Some("Chippin' In")));
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), Some("wav"));
        assert_eq!(sniff(b"PK\x03\x04"), None);

        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("track01.mp3");
        std::fs::write(&p, MP3).unwrap();
        let i = song_info(&p);
        assert_eq!(i.name, "Ada - Night.mp3");
        assert!(i.error.is_none());
        // A FLAC named .mp3 gets the right extension.
        let p = d.path().join("x.mp3");
        std::fs::write(&p, flac()).unwrap();
        let i = song_info(&p);
        assert_eq!(i.name, "Samurai - Chippin' In.flac");
        assert_eq!(i.plain_name, "x.flac");
        let p = d.path().join("cover.png");
        std::fs::write(&p, b"\x89PNG\r\n").unwrap();
        assert!(song_info(&p).error.unwrap().contains("not played by radioExt"));
        let p = d.path().join("fake.ogg");
        std::fs::write(&p, b"hello").unwrap();
        assert!(song_info(&p).error.unwrap().contains("not OGG"));
        assert_eq!(clean_name("AC/DC: Live?", "x"), "AC_DC_ Live_");
        assert_eq!(clean_name("con.mp3", "x"), "_con.mp3");
        assert_eq!(clean_name(" .. ", "x"), "x");
    }

    #[test]
    fn keeps_unknown_metadata_fields() {
        let src = br#"{"displayName":"89.7 Growl","fm":"89.7","volume":1,"icon":"UIIcon.x","customIcon":{"useCustom":"false"},"newField":[1,2]}"#;
        let m = Metadata::parse(src).unwrap();
        assert_eq!(m.fm, 89.7);
        assert!(!m.custom_icon.use_custom);
        assert_eq!(m.extra["newField"], serde_json::json!([1, 2]));
        assert!(m.to_json().unwrap().contains("\"newField\""));
        let mut bad = m.clone();
        bad.fm = 0.0;
        assert!(bad.check().is_err());
        bad = m.clone();
        bad.stream_info.is_stream = true;
        bad.stream_info.stream_url = "ftp://x".into();
        assert!(bad.check().unwrap_err().to_string().contains("http"));
    }

    fn meta(name: &str, fm: f64) -> Metadata {
        let mut m = Metadata::parse(b"{}").unwrap();
        m.display_name = name.into();
        m.fm = fm;
        m.icon = DEFAULT_ICON.into();
        m
    }

    #[test]
    fn creates_edits_and_takes_over_stations() {
        let (dir, db, game) = setup_game();
        let gdir = PathBuf::from(&game.path);
        let inst = installer(&db, dir.path());
        let music = dir.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        for n in ["a.mp3", "b.mp3"] {
            std::fs::write(music.join(n), MP3).unwrap();
        }
        let song = |name: &str, path: &str| DraftSong { name: name.into(), keep: None, path: Some(music.join(path).to_string_lossy().into()) };

        // New station.
        let draft = Draft {
            folder: None,
            metadata: meta("99.1 Night City Nights", 99.1),
            songs: vec![song("One.mp3", "a.mp3"), song("Two.mp3", "b.mp3")],
            fixed_order: true,
            icon_png: None,
        };
        let r = save(&inst, &game, &draft).unwrap();
        let st = gdir.join(RADIOS).join("99.1 Night City Nights");
        assert!(st.join("One.mp3").is_file() && st.join("Two.mp3").is_file());
        let written = Metadata::parse(&std::fs::read(st.join(METADATA)).unwrap()).unwrap();
        assert_eq!(written.order, vec!["One.mp3", "Two.mp3"]);
        assert!(music.join("a.mp3").is_file(), "user files are copied, not moved");
        let s = status(&inst, &game).unwrap();
        assert_eq!(s.stations.len(), 1);
        assert!(s.stations[0].managed && s.stations[0].mod_id == Some(r.mod_id));
        assert_eq!(db.get_mod(r.mod_id).unwrap().source, SOURCE);

        // A second station with the same name gets its own folder.
        let again = save(&inst, &game, &Draft { songs: vec![song("x.mp3", "a.mp3")], ..draft.clone() }).unwrap();
        assert!(gdir.join(RADIOS).join("99.1 Night City Nights (2)").is_dir());
        inst.uninstall(again.mod_id).unwrap();

        // Edit: drop one song, rename the other, shuffle, new name.
        let edit = Draft {
            folder: Some("99.1 Night City Nights".into()),
            metadata: meta("99.2 Nights", 99.2),
            songs: vec![DraftSong { name: "Renamed.mp3".into(), keep: Some("Two.mp3".into()), path: None }],
            fixed_order: false,
            icon_png: None,
        };
        let r2 = save(&inst, &game, &edit).unwrap();
        assert_eq!(r2.replaced.as_deref(), Some("99.1 Night City Nights"));
        assert!(!st.join("One.mp3").exists() && !st.join("Two.mp3").exists());
        assert!(st.join("Renamed.mp3").is_file());
        let s = status(&inst, &game).unwrap();
        assert_eq!(s.stations.len(), 1);
        assert_eq!(s.stations[0].metadata.as_ref().unwrap().display_name, "99.2 Nights");
        assert!(s.stations[0].metadata.as_ref().unwrap().order.is_empty());

        // Disabled stations are listed from their stored copy.
        inst.disable(r2.mod_id).unwrap();
        assert!(!st.exists());
        let s = status(&inst, &game).unwrap();
        assert!(!s.stations[0].enabled && s.stations[0].songs[0].file == "Renamed.mp3");
        inst.enable(r2.mod_id, false).unwrap();

        // A folder no mod owns is taken over on save; uninstall removes it.
        let hand = gdir.join(RADIOS).join("Hand made");
        std::fs::create_dir_all(&hand).unwrap();
        std::fs::write(hand.join(METADATA), meta("88.0 Hand", 88.0).to_json().unwrap()).unwrap();
        std::fs::write(hand.join("song.mp3"), MP3).unwrap();
        std::fs::write(hand.join("cover.png"), b"png").unwrap();
        let s = status(&inst, &game).unwrap();
        let h = s.stations.iter().find(|x| x.folder == "Hand made").unwrap();
        assert!(h.mod_id.is_none() && h.other_files == vec!["cover.png"]);
        let r3 = save(
            &inst,
            &game,
            &Draft {
                folder: Some("Hand made".into()),
                metadata: h.metadata.clone().unwrap(),
                songs: vec![DraftSong { name: "song.mp3".into(), keep: Some("song.mp3".into()), path: None }],
                fixed_order: false,
                icon_png: None,
            },
        )
        .unwrap();
        assert!(hand.join("cover.png").is_file() && hand.join("song.mp3").is_file());
        assert!(r3.backed_up_game_files.is_empty());
        assert!(!gdir.join(RADIOS).join(".cpmx-saving-Hand made").exists());
        inst.uninstall(r3.mod_id).unwrap();
        assert!(!hand.exists());

        // Wrong content is refused before anything changes.
        std::fs::write(music.join("bad.mp3"), b"not audio").unwrap();
        let e = save(&inst, &game, &Draft { songs: vec![song("bad.mp3", "bad.mp3")], ..draft.clone() }).unwrap_err();
        assert!(e.to_string().contains("not MP3"), "{e}");
        let e = save(&inst, &game, &Draft { songs: vec![], ..draft }).unwrap_err();
        assert!(e.to_string().contains("at least one song"), "{e}");
    }

    #[test]
    fn station_icon_from_png() {
        let (dir, db, game) = setup_game();
        let gdir = PathBuf::from(&game.path);
        let inst = installer(&db, dir.path());
        let mp3 = dir.path().join("a.mp3");
        std::fs::write(&mp3, MP3).unwrap();
        let png = dir.path().join("logo.png");
        let img = inkicon::Rgba { width: 64, height: 32, pixels: [10, 200, 30, 255].repeat(64 * 32) };
        std::fs::write(&png, inkicon::encode_png(&img).unwrap()).unwrap();
        let song = DraftSong { name: "a.mp3".into(), keep: None, path: Some(mp3.to_string_lossy().into()) };
        let draft = Draft {
            folder: None,
            metadata: meta("95.5 Ünïcode FM", 95.5),
            songs: vec![song],
            fixed_order: false,
            icon_png: Some(png.to_string_lossy().into()),
        };
        let r = save(&inst, &game, &draft).unwrap();
        let folder = "95.5 Ünïcode FM";
        let rel = icon_archive_rel(folder);
        assert!(rel.starts_with("archive/pc/mod/cpmx2077_radio_95_5_n_code_fm_"), "{rel}");
        assert!(gdir.join(&rel).is_file());
        let written = Metadata::parse(&std::fs::read(gdir.join(RADIOS).join(folder).join(METADATA)).unwrap()).unwrap();
        assert!(written.custom_icon.use_custom);
        assert_eq!(written.custom_icon.ink_atlas_path, icon_atlas_path(folder));
        assert_eq!(written.custom_icon.ink_atlas_part, "icon");
        let s = status(&inst, &game).unwrap();
        assert_eq!(s.stations[0].icon_archive.as_deref(), Some(rel.as_str()));
        assert!(s.stations[0].warnings.is_empty(), "{:?}", s.stations[0].warnings);
        let (_, files) = icon_files(&game, folder).unwrap();
        assert_eq!(files.len(), 2);

        // Editing without a new PNG keeps the icon; disabling removes it.
        let keep = |m: Metadata| Draft {
            folder: Some(folder.into()),
            metadata: m,
            songs: vec![DraftSong { name: "a.mp3".into(), keep: Some("a.mp3".into()), path: None }],
            fixed_order: false,
            icon_png: None,
        };
        let r2 = save(&inst, &game, &keep(written.clone())).unwrap();
        assert_eq!(r2.replaced.as_deref(), Some(folder));
        assert!(gdir.join(&rel).is_file());
        inst.disable(r2.mod_id).unwrap();
        assert!(!gdir.join(&rel).exists());
        inst.enable(r2.mod_id, false).unwrap();
        assert!(gdir.join(&rel).is_file());

        // Back to a UIIcon record: the archive goes.
        let mut plain = written;
        plain.custom_icon.use_custom = false;
        let r3 = save(&inst, &game, &keep(plain)).unwrap();
        assert!(!gdir.join(&rel).exists());
        assert!(icon_files(&game, folder).is_err());
        let _ = r;

        // A file that isn't a PNG is refused.
        std::fs::write(&png, b"GIF89a").unwrap();
        let e = save(&inst, &game, &Draft { icon_png: Some(png.to_string_lossy().into()), ..keep(meta("x", 1.0)) }).unwrap_err();
        assert!(e.to_string().contains("icon: not a PNG"), "{e}");
        inst.uninstall(r3.mod_id).unwrap();
    }

    #[test]
    fn stations_owned_by_other_mods_are_read_only() {
        let (dir, db, game) = setup_game();
        let gdir = PathBuf::from(&game.path);
        let inst = installer(&db, dir.path());
        let arc = dir.path().join("station.zip");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&arc).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file(format!("{RADIOS}/Nexus FM/{METADATA}"), o).unwrap();
            z.write_all(meta("101.0 Nexus FM", 101.0).to_json().unwrap().as_bytes()).unwrap();
            z.start_file(format!("{RADIOS}/Nexus FM/song.mp3"), o).unwrap();
            z.write_all(MP3).unwrap();
            z.finish().unwrap();
        }
        let r = inst.install(&game, &arc, InstallOptions { meta: NewMod { name: "Nexus FM".into(), source: "manual".into(), ..Default::default() }, overwrite: false, fomod_choices: None }).unwrap();
        let s = status(&inst, &game).unwrap();
        assert_eq!(s.stations[0].mod_id, Some(r.mod_id));
        assert!(!s.stations[0].managed);
        let e = save(
            &inst,
            &game,
            &Draft {
                folder: Some("Nexus FM".into()),
                metadata: meta("x", 1.0),
                songs: vec![DraftSong { name: "song.mp3".into(), keep: Some("song.mp3".into()), path: None }],
                fixed_order: false,
                icon_png: None,
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("has files from Nexus FM"), "{e}");
        assert!(gdir.join(RADIOS).join("Nexus FM/song.mp3").is_file());
    }
}
