//! Mod thumbnails for the Installed mods list: each Nexus mod's picture URL
//! (asked from Nexus in batches, kept in the db) and the image itself, saved
//! once under `paths::thumbnails_dir()` so the list shows them offline and
//! without a request per row.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::nexus::now_unix;
use crate::nexus_browse::safe_image;
use crate::{APP_NAME, APP_VERSION, Error, Result};

/// Ask Nexus again after this long, in case the author changed the picture.
pub const URL_TTL_SECS: i64 = 30 * 24 * 3600;
/// Nexus thumbnails are a few dozen KB; anything this big is not one.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Ids from `ids` whose picture URL was never asked for or is older than
/// [`URL_TTL_SECS`].
pub fn stale_ids(db: &Db, ids: &[i64], now: i64) -> Result<Vec<i64>> {
    let mut st = db.conn.prepare("SELECT checked_at FROM mod_thumbnails WHERE nexus_mod_id = ?1")?;
    let mut out = Vec::new();
    for &id in ids {
        let at: Option<i64> = st.query_row([id], |r| r.get(0)).optional()?;
        if at.is_none_or(|at| now - at > URL_TTL_SECS) {
            out.push(id);
        }
    }
    Ok(out)
}

/// Keep what Nexus answered for `asked`. Ids it left out (removed or hidden
/// pages) are kept as "no picture" so they aren't asked for on every load.
pub fn store_urls(db: &Db, asked: &[i64], answered: &[(i64, Option<String>)], now: i64) -> Result<()> {
    let mut st = db.conn.prepare(
        "INSERT INTO mod_thumbnails (nexus_mod_id, url, checked_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(nexus_mod_id) DO UPDATE SET url = excluded.url, checked_at = excluded.checked_at",
    )?;
    for &id in asked {
        let url = answered.iter().find(|(i, _)| *i == id).and_then(|(_, u)| u.as_deref());
        st.execute(params![id, url, now])?;
    }
    Ok(())
}

/// The stored picture URL of each of `ids`; `None` for "no picture" or not
/// asked yet.
pub fn urls(db: &Db, ids: &[i64]) -> Result<Vec<(i64, Option<String>)>> {
    let mut st = db.conn.prepare("SELECT url FROM mod_thumbnails WHERE nexus_mod_id = ?1")?;
    ids.iter()
        .map(|&id| Ok((id, st.query_row([id], |r| r.get::<_, Option<String>>(0)).optional()?.flatten())))
        .collect()
}

/// Where the image for `url` is saved. Named after the URL, so a changed
/// picture is fetched again.
pub fn cache_path(dir: &Path, nexus_mod_id: i64, url: &str) -> PathBuf {
    let h = hex::encode(Sha256::digest(url.as_bytes()));
    dir.join(format!("{nexus_mod_id}-{}", &h[..16]))
}

/// Image type by magic bytes, never by URL or header.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// A saved image and its type, if it is there and still an image.
pub fn read_cached(path: &Path) -> Option<(&'static str, Vec<u8>)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some((sniff(&bytes)?, bytes))
}

/// Download a picture from Nexus' image hosts (HTTPS only, no redirects,
/// size cap, must be an image) and save it at `dest`.
pub fn fetch(url: &str, dest: &Path) -> Result<(&'static str, Vec<u8>)> {
    let url = safe_image(Some(url)).ok_or_else(|| Error::Other(format!("not a Nexus image URL: {url}")))?;
    let http = reqwest::blocking::Client::builder()
        .user_agent(format!("{APP_NAME}/{APP_VERSION}"))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()?;
    download(&http, &url, dest)
}

fn download(http: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<(&'static str, Vec<u8>)> {
    let resp = http.get(url).send()?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("{url} answered {}", resp.status())));
    }
    if resp.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Err(Error::Other(format!("{url} is larger than {MAX_BYTES} bytes")));
    }
    let mut bytes = Vec::new();
    resp.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::Other(format!("{url} is larger than {MAX_BYTES} bytes")));
    }
    let mime = sniff(&bytes).ok_or_else(|| Error::Other(format!("{url} is not a PNG, JPEG, GIF or WebP image")))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write aside, then rename, so a half-written file is never read back.
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, dest)?;
    Ok((mime, bytes))
}

/// Unix seconds, for callers that don't pass their own clock.
pub fn now() -> i64 {
    now_unix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::serve_bytes;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];

    #[test]
    fn sniffs_images_by_content() {
        assert_eq!(sniff(PNG), Some("image/png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a"), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<html>"), None);
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVE"), None);
    }

    #[test]
    fn keeps_urls_and_asks_again_when_old() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(stale_ids(&db, &[1, 2, 3], 1000).unwrap(), [1, 2, 3]);
        store_urls(&db, &[1, 2], &[(1, Some("https://staticdelivery.nexusmods.com/a.png".into())), (9, None)], 1000).unwrap();
        // 2 wasn't answered: stored as "no picture", not asked again.
        assert_eq!(stale_ids(&db, &[1, 2, 3], 1000).unwrap(), [3]);
        assert_eq!(stale_ids(&db, &[1, 2], 1000 + URL_TTL_SECS + 1).unwrap(), [1, 2]);
        let u = urls(&db, &[1, 2, 3]).unwrap();
        assert_eq!(u[0].1.as_deref(), Some("https://staticdelivery.nexusmods.com/a.png"));
        assert_eq!((u[1].1.as_deref(), u[2].1.as_deref()), (None, None));
    }

    #[test]
    fn cache_name_follows_the_url() {
        let d = Path::new("/c");
        assert_eq!(cache_path(d, 7, "https://x/a.png"), cache_path(d, 7, "https://x/a.png"));
        assert_ne!(cache_path(d, 7, "https://x/a.png"), cache_path(d, 7, "https://x/b.png"));
        assert!(cache_path(d, 7, "https://x/a.png").file_name().unwrap().to_string_lossy().starts_with("7-"));
    }

    #[test]
    fn downloads_only_images_within_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let big = [PNG, &vec![0u8; MAX_BYTES as usize]].concat();
        let (addr, _) = serve_bytes(vec![
            ("/ok.png", 200, PNG.to_vec()),
            ("/page.png", 200, b"<html>not an image</html>".to_vec()),
            ("/big.png", 200, big),
            ("/gone.png", 404, vec![]),
        ]);
        let http = reqwest::blocking::Client::new();
        let dest = dir.path().join("t").join("1-x");
        assert_eq!(download(&http, &format!("{addr}/ok.png"), &dest).unwrap().0, "image/png");
        assert_eq!(read_cached(&dest).unwrap().1, PNG);
        for bad in ["page", "big", "gone"] {
            let p = dir.path().join(bad);
            assert!(download(&http, &format!("{addr}/{bad}.png"), &p).is_err(), "{bad}");
            assert!(!p.exists(), "{bad} not saved");
        }
        // Only Nexus' image hosts, over HTTPS.
        assert!(fetch(&format!("{addr}/ok.png"), &dir.path().join("x")).is_err());
        assert!(fetch("https://evil.example/a.png", &dir.path().join("x")).is_err());
    }
}
