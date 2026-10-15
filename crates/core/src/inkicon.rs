//! A PNG turned into a UI icon the game can show: a `CBitmapTexture`
//! (`.xbm`), an `inkTextureAtlas` (`.inkatlas`) with one part covering it,
//! both packed into an RDAR `.archive` for `archive/pc/mod`. radioExt uses
//! this for custom station icons (`customIcon` in a station's metadata).
//!
//! Written from the format notes in the project's research report (byte
//! layouts of WolvenKit's `ArchiveWriter`, `CR2WWriter` and `RedImage`;
//! WolvenKit is GPL-3.0, no code taken). The texture is stored uncompressed
//! (`TCM_None`, `R8G8B8A8_UNORM_SRGB`), so no Oodle and no DirectXTex are
//! needed; segments are wrapped in "stored" Kraken blocks, the form Oodle
//! itself writes for data it doesn't compress. WolvenKit 9.1 reads the result
//! back byte for byte; see [`crate::wolvenkit::check_archive`].

use std::io::{Cursor, Read};
use std::path::Path;

use crate::{Error, Result};

/// Side of the square icon written into the texture.
pub const ICON_SIZE: u32 = 256;
/// Largest PNG file read.
pub const MAX_PNG_BYTES: u64 = 32 << 20;
/// Largest PNG width or height read.
pub const MAX_PNG_SIDE: u32 = 4096;
/// The atlas part name stations point at.
pub const PART: &str = "icon";

// ---- image ------------------------------------------------------------------

/// 8-bit RGBA pixels, rows top to bottom, straight (not premultiplied) alpha.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Width and height from a PNG's IHDR, read before decoding anything.
fn png_size(bytes: &[u8]) -> Result<(u32, u32)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return Err(Error::Other("not a PNG file (signature or IHDR missing)".into()));
    }
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Ok((be(16), be(20)))
}

/// Decode a PNG of any colour type and depth to 8-bit RGBA. The file size and
/// the image size are checked before decoding.
pub fn decode_png(bytes: &[u8]) -> Result<Rgba> {
    if bytes.len() as u64 > MAX_PNG_BYTES {
        return Err(Error::Other(format!("PNG is larger than {} MB", MAX_PNG_BYTES >> 20)));
    }
    let (w, h) = png_size(bytes)?;
    if w == 0 || h == 0 || w > MAX_PNG_SIDE || h > MAX_PNG_SIDE {
        return Err(Error::Other(format!("PNG is {w}×{h}; up to {MAX_PNG_SIDE}×{MAX_PNG_SIDE} accepted")));
    }
    let bad = |e: png::DecodingError| Error::Other(format!("PNG not readable: {e}"));
    let limits = png::Limits { bytes: (MAX_PNG_SIDE as usize).pow(2) * 8 + (1 << 20) };
    let mut dec = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(bad)?;
    let size = reader.output_buffer_size().ok_or_else(|| Error::Other("PNG too large to decode".into()))?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(bad)?;
    buf.truncate(info.buffer_size());
    let (width, height) = (info.width, info.height);
    let n = (width * height) as usize;
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(Error::Other("PNG palette was not expanded".into())),
    };
    if pixels.len() != n * 4 {
        return Err(Error::Other("PNG pixel data has the wrong size".into()));
    }
    Ok(Rgba { width, height, pixels })
}

pub fn load_png(path: &Path) -> Result<Rgba> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX_PNG_BYTES + 1).read_to_end(&mut bytes)?;
    decode_png(&bytes)
}

/// The image centred on a transparent square of side `size`, scaled so its
/// longer side fills it. Scaling averages the source area under each output
/// pixel (bilinear when enlarging), in premultiplied alpha so transparent
/// pixels don't darken the edges.
pub fn fit_square(img: &Rgba, size: u32) -> Rgba {
    let (sw, sh) = (img.width as f64, img.height as f64);
    let scale = size as f64 / sw.max(sh);
    let (dw, dh) = (sw * scale, sh * scale);
    let (ox, oy) = ((size as f64 - dw) / 2.0, (size as f64 - dh) / 2.0);
    let src = |x: i64, y: i64| -> [f64; 4] {
        let x = x.clamp(0, img.width as i64 - 1) as usize;
        let y = y.clamp(0, img.height as i64 - 1) as usize;
        let p = &img.pixels[(y * img.width as usize + x) * 4..][..4];
        let a = p[3] as f64 / 255.0;
        [p[0] as f64 * a, p[1] as f64 * a, p[2] as f64 * a, a]
    };
    // Bilinear sample at a source position (pixel centres at .5).
    let sample = |x: f64, y: f64| -> [f64; 4] {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (src(x0, y0), src(x0 + 1, y0), src(x0, y0 + 1), src(x0 + 1, y0 + 1));
        std::array::from_fn(|i| (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty)
    };
    // Samples per output pixel and axis: enough to cover the source area.
    let k = (1.0 / scale).ceil().max(1.0) as usize;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            if px < ox || px > ox + dw || py < oy || py > oy + dh {
                continue;
            }
            let mut acc = [0.0; 4];
            for j in 0..k {
                for i in 0..k {
                    let sx = (x as f64 + (i as f64 + 0.5) / k as f64 - ox) / scale;
                    let sy = (y as f64 + (j as f64 + 0.5) / k as f64 - oy) / scale;
                    let s = sample(sx, sy);
                    for c in 0..4 {
                        acc[c] += s[c];
                    }
                }
            }
            let a = acc[3] / (k * k) as f64;
            let o = ((y * size + x) * 4) as usize;
            if a > 0.0 {
                for c in 0..3 {
                    out[o + c] = (acc[c] / (k * k) as f64 / a).round().clamp(0.0, 255.0) as u8;
                }
            }
            out[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Rgba { width: size, height: size, pixels: out }
}

/// The image as a PNG (for previews).
pub fn encode_png(img: &Rgba) -> Result<Vec<u8>> {
    let bad = |e: png::EncodingError| Error::Other(format!("PNG not written: {e}"));
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(bad)?;
    w.write_image_data(&img.pixels).map_err(bad)?;
    w.finish().map_err(bad)?;
    Ok(out)
}

// ---- hashes -------------------------------------------------------------------

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// CRC-64/XZ, the checksum over an archive's file table.
pub fn crc64_xz(bytes: &[u8]) -> u64 {
    const POLY: u64 = 0xc96c_5795_d787_0f42;
    let mut crc = !0u64;
    for &b in bytes {
        crc ^= b as u64;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ POLY } else { crc >> 1 };
        }
    }
    !crc
}

fn crc32(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

/// A depot path the way the game hashes it: trimmed, lowercase, backslashes.
pub fn sanitize_path(p: &str) -> String {
    let mut s = p.trim_matches(|c| matches!(c, '\'' | '"' | '/' | '\\' | ' ' | '\n' | '\r')).to_lowercase().replace('/', "\\");
    while s.contains("\\\\") {
        s = s.replace("\\\\", "\\");
    }
    s
}

pub fn path_hash(p: &str) -> u64 {
    fnv1a64(sanitize_path(p).as_bytes())
}

/// Wrap bytes in Kraken "stored" blocks (`0xCC 0x06` + up to 256 KiB raw),
/// which the game's Oodle decoder copies as is.
pub fn kark_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 8 + 2 * (raw.len() / 0x40000 + 1));
    out.extend_from_slice(b"KARK");
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    for block in raw.chunks(0x40000) {
        out.extend_from_slice(&[0xCC, 0x06]);
        out.extend_from_slice(block);
    }
    out
}

// ---- CR2W -------------------------------------------------------------------

/// How segments and buffers are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// Uncompressed (`zSize == size`).
    Raw,
    /// Kraken "stored" blocks: shaped like every vanilla compressed segment.
    Stored,
}

fn le16(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

/// Names, imports and property encoders of one CR2W file. Names are indexed
/// in order of first use.
#[derive(Default)]
struct Writer {
    names: Vec<String>,
    imports: Vec<String>,
}

impl Writer {
    fn new() -> Self {
        Writer { names: vec![String::new()], imports: vec![] }
    }

    fn n(&mut self, s: &str) -> u16 {
        if let Some(i) = self.names.iter().position(|x| x == s) {
            return i as u16;
        }
        self.names.push(s.to_string());
        (self.names.len() - 1) as u16
    }

    /// 1-based import index of a depot path.
    fn import(&mut self, path: &str) -> u16 {
        if let Some(i) = self.imports.iter().position(|x| x == path) {
            return i as u16 + 1;
        }
        self.imports.push(path.to_string());
        self.imports.len() as u16
    }

    fn head(&mut self, name: &str, typ: &str, payload_len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + payload_len);
        out.extend_from_slice(&le16(self.n(name)));
        out.extend_from_slice(&le16(self.n(typ)));
        out.extend_from_slice(&(4 + payload_len as u32).to_le_bytes());
        out
    }

    fn prop(&mut self, name: &str, typ: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = self.head(name, typ, payload.len());
        out.extend_from_slice(payload);
        out
    }

    fn u32(&mut self, name: &str, v: u32) -> Vec<u8> {
        self.prop(name, "Uint32", &v.to_le_bytes())
    }
    fn u16(&mut self, name: &str, v: u16) -> Vec<u8> {
        self.prop(name, "Uint16", &v.to_le_bytes())
    }
    fn u8(&mut self, name: &str, v: u8) -> Vec<u8> {
        self.prop(name, "Uint8", &[v])
    }
    fn bool(&mut self, name: &str, v: bool) -> Vec<u8> {
        self.prop(name, "Bool", &[v as u8])
    }
    fn f32(&mut self, name: &str, v: f32) -> Vec<u8> {
        self.prop(name, "Float", &v.to_le_bytes())
    }
    /// An enum or CName value: the value's name index.
    fn named(&mut self, name: &str, typ: &str, value: &str) -> Vec<u8> {
        let mut out = self.head(name, typ, 2);
        out.extend_from_slice(&le16(self.n(value)));
        out
    }
    /// A class body: `u8 0`, the properties, `u16 0`.
    fn class(props: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = vec![0u8];
        for p in props {
            out.extend_from_slice(&p);
        }
        out.extend_from_slice(&[0, 0]);
        out
    }
    /// A nested struct property; `props` are encoded first (name order).
    fn sub(&mut self, name: &str, typ: &str, props: Vec<Vec<u8>>) -> Vec<u8> {
        let body = Self::class(props);
        self.prop(name, typ, &body)
    }
}

/// A CR2W file and what the archive needs to split it into segments.
#[derive(Debug, Clone)]
pub struct Cr2w {
    pub bytes: Vec<u8>,
    /// End of the header and objects; segment 0 of the archive entry.
    pub objects_end: usize,
    /// Each buffer's on-disk and in-memory size, in file order.
    pub buffers: Vec<(u32, u32)>,
}

fn cr2w(w: &Writer, chunks: &[(&str, Vec<u8>)], buffers: &[Vec<u8>], storage: Storage) -> Cr2w {
    let mut pool = Vec::new();
    let mut name_off = Vec::new();
    for n in &w.names {
        name_off.push(pool.len() as u32);
        pool.extend_from_slice(n.as_bytes());
        pool.push(0);
    }
    let mut imp_off = Vec::new();
    for p in &w.imports {
        imp_off.push(pool.len() as u32);
        pool.extend_from_slice(p.as_bytes());
        pool.push(0);
    }
    let mut names_t = Vec::new();
    for (n, off) in w.names.iter().zip(&name_off) {
        let f = fnv1a64(n.as_bytes());
        let hash = if n.is_empty() { 0 } else { ((f >> 32) as u32) ^ (f as u32) };
        names_t.extend_from_slice(&off.to_le_bytes());
        names_t.extend_from_slice(&hash.to_le_bytes());
    }
    let mut imps_t = Vec::new();
    for off in &imp_off {
        imps_t.extend_from_slice(&off.to_le_bytes());
        imps_t.extend_from_slice(&0u16.to_le_bytes());
        imps_t.extend_from_slice(&4u16.to_le_bytes()); // Soft
    }
    let props_t = [0u8; 16];
    let pos = 160 + pool.len();
    let after = pos + names_t.len() + imps_t.len() + props_t.len() + 24 * chunks.len() + 24 * buffers.len();
    let mut exps = Vec::new();
    let mut data = Vec::new();
    for (class, body) in chunks {
        let idx = w.names.iter().position(|x| x == class).unwrap_or(0) as u16;
        exps.extend_from_slice(&idx.to_le_bytes());
        exps.extend_from_slice(&0u16.to_le_bytes());
        for v in [0u32, body.len() as u32, (after + data.len()) as u32, 0, 0] {
            exps.extend_from_slice(&v.to_le_bytes());
        }
        data.extend_from_slice(body);
    }
    let objects_end = after + data.len();
    let mut bufs_t = Vec::new();
    let mut buf_data = Vec::new();
    let mut sizes = Vec::new();
    for raw in buffers {
        let disk = match storage {
            Storage::Raw => raw.clone(),
            Storage::Stored => kark_stored(raw),
        };
        for v in [0x20000u32, 0, (objects_end + buf_data.len()) as u32, disk.len() as u32, raw.len() as u32, crc32(&disk)] {
            bufs_t.extend_from_slice(&v.to_le_bytes());
        }
        sizes.push((disk.len() as u32, raw.len() as u32));
        buf_data.extend_from_slice(&disk);
    }
    // Table headers: strings, names, imports, properties, exports, buffers.
    let mut tables = vec![(160u32, pool.len() as u32, crc32(&pool))];
    let mut p = pos;
    for (t, count) in [
        (&names_t, w.names.len()),
        (&imps_t, w.imports.len()),
        (&props_t.to_vec(), 1),
        (&exps, chunks.len()),
        (&bufs_t, buffers.len()),
    ] {
        tables.push(if count > 0 { (p as u32, count as u32, crc32(t)) } else { (0, 0, 0) });
        p += t.len();
    }
    tables.resize(10, (0, 0, 0));
    let mut tb = Vec::new();
    for (a, b, c) in &tables {
        for v in [a, b, c] {
            tb.extend_from_slice(&v.to_le_bytes());
        }
    }
    let buffers_end = objects_end + buf_data.len();
    let mut fields = Vec::new();
    fields.extend_from_slice(&195u32.to_le_bytes());
    fields.extend_from_slice(&0u32.to_le_bytes());
    fields.extend_from_slice(&0u64.to_le_bytes());
    fields.extend_from_slice(&0u32.to_le_bytes());
    fields.extend_from_slice(&(objects_end as u32).to_le_bytes());
    fields.extend_from_slice(&(buffers_end as u32).to_le_bytes());
    let mut crc_input = b"CR2W".to_vec();
    crc_input.extend_from_slice(&fields);
    crc_input.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    crc_input.extend_from_slice(&6u32.to_le_bytes());
    crc_input.extend_from_slice(&tb);
    let mut out = b"CR2W".to_vec();
    out.extend_from_slice(&fields);
    out.extend_from_slice(&crc32(&crc_input).to_le_bytes());
    out.extend_from_slice(&6u32.to_le_bytes());
    out.extend_from_slice(&tb);
    for part in [&pool[..], &names_t, &imps_t, &props_t, &exps, &bufs_t, &data, &buf_data] {
        out.extend_from_slice(part);
    }
    debug_assert_eq!(out.len(), buffers_end);
    Cr2w { bytes: out, objects_end, buffers: sizes }
}

/// A `CBitmapTexture` holding `img` uncompressed (rows stored bottom-up, as
/// the game expects).
pub fn xbm(img: &Rgba, storage: Storage) -> Cr2w {
    let (wd, ht) = (img.width, img.height);
    let row = wd as usize * 4;
    let data: Vec<u8> = img.pixels.chunks_exact(row).rev().flatten().copied().collect();
    let len = data.len() as u32;
    let mut w = Writer::new();
    w.n("CBitmapTexture");
    let c0 = {
        let cp = w.named("cookingPlatform", "ECookingPlatform", "PLATFORM_PC");
        let width = w.u32("width", wd);
        let height = w.u32("height", ht);
        let setup_props = vec![
            w.named("group", "GpuWrapApieTextureGroup", "TEXG_Generic_UI"),
            w.bool("isStreamable", false),
            w.bool("hasMipchain", false),
            w.bool("isGamma", true),
            w.bool("allowTextureDowngrade", false),
        ];
        let setup = w.sub("setup", "STextureGroupSetup", setup_props);
        let blob = vec![w.prop("renderResourceBlobPC", "handle:IRenderResourceBlob", &2i32.to_le_bytes())];
        let rtr = w.sub("renderTextureResource", "rendRenderTextureResource", blob);
        Writer::class(vec![cp, width, height, setup, rtr])
    };
    w.n("rendRenderTextureBlobPC");
    let c1 = {
        let version = w.u32("version", 2);
        let size_props = vec![w.u16("width", wd as u16), w.u16("height", ht as u16)];
        let size_info = w.sub("sizeInfo", "rendRenderTextureBlobSizeInfo", size_props);
        let tex_props = vec![
            w.u32("textureDataSize", len),
            w.u32("sliceSize", len),
            w.u32("dataAlignment", 8),
            w.u16("sliceCount", 1),
            w.u8("mipCount", 1),
        ];
        let tex_info = w.sub("textureInfo", "rendRenderTextureBlobTextureInfo", tex_props);
        let layout_props = vec![w.u32("rowPitch", wd * 4), w.u32("slicePitch", len)];
        let layout = w.sub("layout", "rendRenderTextureBlobMemoryLayout", layout_props);
        let placement_props = vec![w.u32("size", len)];
        let placement = w.sub("placement", "rendRenderTextureBlobPlacement", placement_props);
        let mut arr = 1u32.to_le_bytes().to_vec();
        arr.extend_from_slice(&Writer::class(vec![layout, placement]));
        let mips = w.prop("mipMapInfo", "array:rendRenderTextureBlobMipMapInfo", &arr);
        let flags = w.u32("flags", 1);
        let header = w.sub("header", "rendRenderTextureBlobHeader", vec![version, size_info, tex_info, mips, flags]);
        let tex = w.prop("textureData", "serializationDeferredDataBuffer", &le16(1));
        Writer::class(vec![header, tex])
    };
    cr2w(&w, &[("CBitmapTexture", c0), ("rendRenderTextureBlobPC", c1)], &[data], storage)
}

/// An `inkTextureAtlas` with one part `part` covering all of `xbm_path`.
pub fn inkatlas(xbm_path: &str, part: &str, storage: Storage) -> Cr2w {
    let mut w = Writer::new();
    w.n("inkTextureAtlas");
    let cp = w.named("cookingPlatform", "ECookingPlatform", "PLATFORM_PC");
    let (ni, ti) = (w.n("slots"), w.n("[3]inkTextureSlot"));
    let slot = |w: &mut Writer| {
        let part_name = w.named("partName", "CName", part);
        let rect_props = vec![w.f32("Left", 0.0), w.f32("Top", 0.0), w.f32("Right", 1.0), w.f32("Bottom", 1.0)];
        let rect = w.sub("clippingRectInUVCoords", "RectF", rect_props);
        let mut arr = 1u32.to_le_bytes().to_vec();
        arr.extend_from_slice(&Writer::class(vec![part_name, rect]));
        let imp = w.import(xbm_path);
        let texture = w.prop("texture", "raRef:CBitmapTexture", &le16(imp));
        let parts = w.prop("parts", "array:inkTextureAtlasMapper", &arr);
        Writer::class(vec![texture, parts])
    };
    let mut body = 3u32.to_le_bytes().to_vec();
    body.extend_from_slice(&slot(&mut w));
    body.extend_from_slice(&slot(&mut w));
    body.extend_from_slice(&Writer::class(vec![]));
    let mut slots = Vec::new();
    slots.extend_from_slice(&le16(ni));
    slots.extend_from_slice(&le16(ti));
    slots.extend_from_slice(&(4 + body.len() as u32).to_le_bytes());
    slots.extend_from_slice(&body);
    let c0 = Writer::class(vec![cp, slots]);
    cr2w(&w, &[("inkTextureAtlas", c0)], &[], storage)
}

// ---- archive ------------------------------------------------------------------

/// An RDAR (version 12) archive of CR2W files, with their depot paths listed
/// (LXRS block) so tools show names.
pub fn rdar(files: &[(&str, &Cr2w)], storage: Storage) -> Vec<u8> {
    let mut entries: Vec<(u64, String, &Cr2w)> = files.iter().map(|(p, f)| (path_hash(p), sanitize_path(p), *f)).collect();
    entries.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let mut paths = Vec::new();
    for (_, p, _) in &entries {
        paths.extend(p.chars().map(|c| c as u32 as u8));
        paths.push(0);
    }
    let mut lxrs = Vec::new();
    for v in [0x4C58_5253u32, 1] {
        lxrs.extend_from_slice(&v.to_le_bytes());
    }
    for v in [paths.len() as i32, paths.len() as i32, entries.len() as i32] {
        lxrs.extend_from_slice(&v.to_le_bytes());
    }
    lxrs.extend_from_slice(&paths);

    let mut out = vec![0u8; 172];
    out.extend_from_slice(&lxrs);
    let mut segs: Vec<(u64, u32, u32)> = Vec::new();
    let mut table_entries = Vec::new();
    for (hash, _, f) in &entries {
        let first = segs.len() as u32;
        let head = &f.bytes[..f.objects_end];
        let main = match storage {
            Storage::Raw => head.to_vec(),
            Storage::Stored => kark_stored(head),
        };
        segs.push((out.len() as u64, main.len() as u32, f.objects_end as u32));
        out.extend_from_slice(&main);
        let mut q = f.objects_end;
        for (disk, mem) in &f.buffers {
            segs.push((out.len() as u64, *disk, *mem));
            out.extend_from_slice(&f.bytes[q..q + *disk as usize]);
            q += *disk as usize;
        }
        table_entries.push((*hash, f.buffers.len().saturating_sub(1) as u32, first, segs.len() as u32));
    }
    out.resize(out.len().next_multiple_of(4096), 0);
    let mut tbl = Vec::new();
    for v in [table_entries.len() as u32, segs.len() as u32, 0] {
        tbl.extend_from_slice(&v.to_le_bytes());
    }
    for (hash, inline, start, end) in &table_entries {
        tbl.extend_from_slice(&hash.to_le_bytes());
        tbl.extend_from_slice(&0i64.to_le_bytes());
        for v in [*inline, *start, *end, 0, 0] {
            tbl.extend_from_slice(&v.to_le_bytes());
        }
        tbl.extend_from_slice(&[0u8; 20]);
    }
    for (off, z, s) in &segs {
        tbl.extend_from_slice(&off.to_le_bytes());
        tbl.extend_from_slice(&z.to_le_bytes());
        tbl.extend_from_slice(&s.to_le_bytes());
    }
    let ipos = out.len();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(tbl.len() as u32 + 8).to_le_bytes());
    out.extend_from_slice(&crc64_xz(&tbl).to_le_bytes());
    out.extend_from_slice(&tbl);
    let isz = out.len() - ipos;
    out.resize(out.len().next_multiple_of(4096), 0);
    let total = out.len() as u64;
    let mut hdr = Vec::with_capacity(44);
    hdr.extend_from_slice(&0x5241_4452u32.to_le_bytes());
    hdr.extend_from_slice(&12u32.to_le_bytes());
    hdr.extend_from_slice(&(ipos as u64).to_le_bytes());
    hdr.extend_from_slice(&(isz as u32).to_le_bytes());
    hdr.extend_from_slice(&0u64.to_le_bytes());
    hdr.extend_from_slice(&0u32.to_le_bytes());
    hdr.extend_from_slice(&total.to_le_bytes());
    hdr.extend_from_slice(&(lxrs.len() as u32).to_le_bytes());
    out[..44].copy_from_slice(&hdr);
    out
}

// ---- icons ------------------------------------------------------------------------

/// An icon ready to install.
#[derive(Debug, Clone)]
pub struct Icon {
    /// Depot path of the texture.
    pub xbm_path: String,
    /// Depot path of the atlas (what radioExt's `inkAtlasPath` names).
    pub atlas_path: String,
    pub part: &'static str,
    pub xbm: Cr2w,
    pub inkatlas: Cr2w,
    pub archive: Vec<u8>,
}

/// Build the texture, atlas and archive for `img` (any size) under the depot
/// folder `dir` (ASCII, backslashes).
pub fn build(img: &Rgba, dir: &str) -> Result<Icon> {
    if !dir.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '\\')) || dir.is_empty() {
        return Err(Error::Other(format!("depot folder {dir} must be lowercase ASCII letters, digits, _ and \\")));
    }
    let square = if img.width == ICON_SIZE && img.height == ICON_SIZE { img.clone() } else { fit_square(img, ICON_SIZE) };
    let xbm_path = format!("{dir}\\icon.xbm");
    let atlas_path = format!("{dir}\\icon.inkatlas");
    let x = xbm(&square, Storage::Stored);
    let a = inkatlas(&xbm_path, PART, Storage::Stored);
    let archive = rdar(&[(&xbm_path, &x), (&atlas_path, &a)], Storage::Stored);
    Ok(Icon { xbm_path, atlas_path, part: PART, xbm: x, inkatlas: a, archive })
}

/// Read back the files of an archive written by [`rdar`]: (path hash, file
/// bytes as a tool extracts them). Stored Kraken blocks are unwrapped;
/// anything really compressed is refused.
pub fn read_archive(bytes: &[u8]) -> Result<Vec<(u64, Vec<u8>)>> {
    let bad = |what: &str| Error::Other(format!("archive: {what}"));
    let u32_at = |i: usize| -> Result<u32> { Ok(u32::from_le_bytes(bytes.get(i..i + 4).ok_or_else(|| bad("truncated"))?.try_into().unwrap())) };
    let u64_at = |i: usize| -> Result<u64> { Ok(u64::from_le_bytes(bytes.get(i..i + 8).ok_or_else(|| bad("truncated"))?.try_into().unwrap())) };
    if u32_at(0)? != 0x5241_4452 || u32_at(4)? != 12 {
        return Err(bad("not an RDAR v12 archive"));
    }
    let ipos = u64_at(8)? as usize;
    let table_len = u32_at(ipos + 4)? as usize - 8;
    let table = bytes.get(ipos + 16..ipos + 16 + table_len).ok_or_else(|| bad("truncated index"))?;
    if crc64_xz(table) != u64_at(ipos + 8)? {
        return Err(bad("index checksum mismatch"));
    }
    let t = ipos + 16;
    let (n_files, n_segs) = (u32_at(t)? as usize, u32_at(t + 4)? as usize);
    let seg_base = t + 12 + n_files * 56;
    let unwrap = |seg: &[u8], size: usize| -> Result<Vec<u8>> {
        if seg.len() == size {
            return Ok(seg.to_vec());
        }
        if seg.get(..4) != Some(b"KARK") {
            return Err(bad("compressed segment"));
        }
        let mut out = Vec::with_capacity(size);
        let mut i = 8;
        while out.len() < size {
            if seg.get(i..i + 2) != Some(&[0xCC, 0x06]) {
                return Err(bad("compressed Kraken block"));
            }
            let n = (size - out.len()).min(0x40000);
            out.extend_from_slice(seg.get(i + 2..i + 2 + n).ok_or_else(|| bad("truncated block"))?);
            i += 2 + n;
        }
        Ok(out)
    };
    let mut files = Vec::new();
    for e in 0..n_files {
        let at = t + 12 + e * 56;
        let hash = u64_at(at)?;
        let (start, end) = (u32_at(at + 20)? as usize, u32_at(at + 24)? as usize);
        if start >= end || end > n_segs {
            return Err(bad("bad segment range"));
        }
        let mut file = Vec::new();
        for s in start..end {
            let sa = seg_base + s * 16;
            let (off, z, size) = (u64_at(sa)? as usize, u32_at(sa + 8)? as usize, u32_at(sa + 12)? as usize);
            let seg = bytes.get(off..off + z).ok_or_else(|| bad("segment outside the file"))?;
            // Segment 0 is the CR2W head (decompressed); buffers stay as on disk.
            if s == start { file.extend(unwrap(seg, size)?) } else { file.extend_from_slice(seg) }
        }
        files.push((hash, file));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/inkicon");
    const XBM: &str = "cpmx2077\\radioext\\teststation\\icon.xbm";
    const ATLAS: &str = "cpmx2077\\radioext\\teststation\\icon.inkatlas";

    /// The research prototype's 64×64 test pattern.
    fn pattern() -> Rgba {
        let mut pixels = Vec::new();
        for y in 0..64u32 {
            for x in 0..64u32 {
                pixels.extend([(x * 4) as u8, (y * 4) as u8, 128, if (x / 8 + y / 8) % 2 == 1 { 255 } else { 128 }]);
            }
        }
        Rgba { width: 64, height: 64, pixels }
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIX}/{name}")).unwrap()
    }

    #[test]
    fn known_hashes() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        // CRC-64/XZ check value.
        assert_eq!(crc64_xz(b"123456789"), 0x995d_c9bb_df19_39fa);
        assert_eq!(sanitize_path(" /Base//Gameplay/x.xbm "), "base\\gameplay\\x.xbm");
        assert_eq!(path_hash("A/B.xbm"), crate::analysis::depot_hash("a\\b.xbm"));
    }

    /// Byte-identical to the research prototype's output, which WolvenKit 9.1
    /// read back to the same JSON as files WolvenKit made itself.
    #[test]
    fn matches_checked_reference_files() {
        for (storage, dir) in [(Storage::Raw, "raw"), (Storage::Stored, "stored")] {
            let x = xbm(&pattern(), storage);
            let a = inkatlas(XBM, PART, storage);
            assert_eq!(x.bytes, fixture(&format!("{dir}/icon.xbm")), "{dir} xbm");
            assert_eq!(a.bytes, fixture(&format!("{dir}/icon.inkatlas")), "{dir} inkatlas");
            assert_eq!(rdar(&[(XBM, &x), (ATLAS, &a)], storage), fixture(&format!("{dir}/proto.archive")), "{dir} archive");
        }
    }

    #[test]
    fn archive_reads_back() {
        let icon = build(&pattern(), "cpmx2077\\radioext\\teststation").unwrap();
        let files = read_archive(&icon.archive).unwrap();
        assert_eq!(files.len(), 2);
        let get = |p: &str| files.iter().find(|(h, _)| *h == path_hash(p)).map(|(_, b)| b.clone()).unwrap();
        assert_eq!(get(XBM), icon.xbm.bytes);
        assert_eq!(get(ATLAS), icon.inkatlas.bytes);
        // The analysis index sees both resources.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.archive");
        std::fs::write(&p, &icon.archive).unwrap();
        let mut hashes = crate::analysis::archive_hashes(&p).unwrap();
        hashes.sort();
        let mut want = vec![path_hash(XBM), path_hash(ATLAS)];
        want.sort();
        assert_eq!(hashes, want);
        // 256×256 RGBA plus headers.
        assert!(icon.xbm.bytes.len() > 256 * 256 * 4);
        assert!(build(&pattern(), "Bad Name").is_err());
    }

    #[test]
    fn xbm_is_bottom_up() {
        let img = Rgba { width: 2, height: 2, pixels: vec![1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4] };
        let x = xbm(&img, Storage::Raw);
        assert_eq!(&x.bytes[x.bytes.len() - 16..], &[3, 3, 3, 3, 4, 4, 4, 4, 1, 1, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn decodes_png_types_and_refuses_bad_input() {
        let img = pattern();
        let png = encode_png(&img).unwrap();
        assert_eq!(decode_png(&png).unwrap(), img);
        // Grey and RGB expand to RGBA.
        for (color, px) in [(png::ColorType::Grayscale, vec![7u8; 4]), (png::ColorType::Rgb, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])] {
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, 2, 2);
            e.set_color(color);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&px).unwrap();
            w.finish().unwrap();
            let d = decode_png(&out).unwrap();
            assert_eq!((d.width, d.height, d.pixels.len()), (2, 2, 16));
            assert_eq!(d.pixels[3], 255);
        }
        assert!(decode_png(b"not a png at all, just text").is_err());
        // An IHDR claiming a huge image is refused before decoding.
        let mut huge = png.clone();
        huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(decode_png(&huge).unwrap_err().to_string().contains("4096"));
    }

    #[test]
    fn fits_into_a_square() {
        // A 4×2 opaque red image: centred, transparent bands above and below.
        let img = Rgba { width: 4, height: 2, pixels: [255, 0, 0, 255].repeat(8) };
        let sq = fit_square(&img, 8);
        assert_eq!((sq.width, sq.height), (8, 8));
        let at = |x: usize, y: usize| &sq.pixels[(y * 8 + x) * 4..][..4];
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(at(4, 4), &[255, 0, 0, 255]);
        assert_eq!(at(4, 7)[3], 0);
        // Downscaling averages.
        let big = Rgba { width: 512, height: 512, pixels: [0, 255, 0, 255].repeat(512 * 512) };
        let small = fit_square(&big, ICON_SIZE);
        assert!(small.pixels.chunks(4).all(|p| p == [0, 255, 0, 255]));
    }
}
