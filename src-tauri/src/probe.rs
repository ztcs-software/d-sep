//! Detects the DJI log profile of a clip.
//!
//! DJI tags every clip as Rec.709 in `colr`, even when it was shot in log. The
//! real profile lives in a QuickTime `keys`/`ilst` pair inside `moov/meta`, as
//! `com.dji.camera.ColorGammaSxS` = "D-Log", "D-Log2", "D-Log M", …
//!
//! We walk box headers with seeks instead of reading the file, so a probe costs
//! a handful of tiny reads no matter how long the clip is.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;

pub const GAMMA_KEY: &str = "com.dji.camera.ColorGammaSxS";

/// A `keys` box is a few hundred bytes and the profile's `ilst` item a few dozen,
/// so anything bigger isn't what we're after.
const MAX_READ_BYTES: u64 = 64 * 1024;
/// How much of the file end to search when the box chain is unreadable.
const TAIL_SCAN_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BOXES: usize = 4096;
/// Longest profile name kept for display; a corrupt file could hold megabytes.
const MAX_LABEL_CHARS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "label", rename_all = "lowercase")]
pub enum Profile {
    DLog,
    DLog2,
    DLogM,
    /// Another DJI profile ("Rec.2100 HLG", "Rec.709", …), kept verbatim.
    Other(String),
    /// No DJI color metadata: not a DJI clip, or a camera that doesn't write it.
    Unknown,
}

impl Profile {
    pub fn from_gamma(raw: &str) -> Self {
        let raw = raw.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        let key: String = raw
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match key.as_str() {
            "" => Profile::Unknown,
            "dlog" => Profile::DLog,
            "dlog2" => Profile::DLog2,
            "dlogm" => Profile::DLogM,
            _ => Profile::Other(label(raw)),
        }
    }
}

/// `raw` without control characters, cut to `MAX_LABEL_CHARS`.
fn label(raw: &str) -> String {
    let mut chars = raw.chars().filter(|c| !c.is_control());
    let mut label: String = chars.by_ref().take(MAX_LABEL_CHARS).collect();
    if chars.next().is_some() {
        label.push('…');
    }
    label
}

pub fn detect(path: &Path) -> io::Result<Profile> {
    Ok(read_gamma(path)?.map_or(Profile::Unknown, |g| Profile::from_gamma(&g)))
}

/// Returns the raw `ColorGammaSxS` value, if the file has one.
pub fn read_gamma(path: &Path) -> io::Result<Option<String>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let top = children(&mut file, 0, len)?;
    match top.iter().find(|b| &b.kind == b"moov") {
        Some(moov) => gamma_in_moov(&mut file, moov),
        // The box chain is broken (e.g. a mangled `mdat` size): search near EOF.
        None => gamma_in_tail(&mut file, len),
    }
}

struct BoxRef {
    kind: [u8; 4],
    payload: u64,
    end: u64,
}

fn gamma_in_moov(file: &mut File, moov: &BoxRef) -> io::Result<Option<String>> {
    let kids = children(file, moov.payload, moov.end)?;
    // DJI writes it in moov/meta; moov/udta/meta is checked in case other firmware moves it.
    for meta in kids.iter().filter(|b| &b.kind == b"meta") {
        if let Some(gamma) = gamma_in_meta_box(file, meta)? {
            return Ok(Some(gamma));
        }
    }
    for udta in kids.iter().filter(|b| &b.kind == b"udta") {
        for meta in children(file, udta.payload, udta.end)?.iter().filter(|b| &b.kind == b"meta") {
            if let Some(gamma) = gamma_in_meta_box(file, meta)? {
                return Ok(Some(gamma));
            }
        }
    }
    Ok(None)
}

/// Reads only `keys` and the profile's `ilst` item, so a `meta` of any size costs a few small reads,
/// and thumbnails stored next to the profile are never read.
fn gamma_in_meta_box(file: &mut File, meta: &BoxRef) -> io::Result<Option<String>> {
    // ISO-style `meta` starts with version/flags; QuickTime-style starts right away with a child box.
    let mut head = [0xFF; 4];
    if meta.end - meta.payload >= 4 {
        file.seek(SeekFrom::Start(meta.payload))?;
        file.read_exact(&mut head)?;
    }
    let start = if head == [0; 4] { meta.payload + 4 } else { meta.payload };
    let kids = children(file, start, meta.end)?;
    let find = |kind: &[u8; 4]| kids.iter().find(|b| &b.kind == kind);
    let (Some(keys), Some(ilst)) = (find(b"keys"), find(b"ilst")) else { return Ok(None) };
    let Some(index) = read_payload(file, keys)?.as_deref().and_then(key_index) else { return Ok(None) };
    let items = children(file, ilst.payload, ilst.end)?;
    let Some(item) = items.iter().find(|b| u32::from_be_bytes(b.kind) == index) else { return Ok(None) };
    Ok(read_payload(file, item)?.as_deref().and_then(item_value))
}

/// A box's payload, or None when it's too big to be one we want.
fn read_payload(file: &mut File, found: &BoxRef) -> io::Result<Option<Vec<u8>>> {
    let len = found.end - found.payload;
    if len > MAX_READ_BYTES {
        return Ok(None);
    }
    let mut buf = vec![0; len as usize];
    file.seek(SeekFrom::Start(found.payload))?;
    file.read_exact(&mut buf)?;
    Ok(Some(buf))
}

fn gamma_in_tail(file: &mut File, len: u64) -> io::Result<Option<String>> {
    let start = len.saturating_sub(TAIL_SCAN_BYTES);
    let mut buf = vec![0; (len - start) as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut buf)?;
    Ok(gamma_in_bytes(&buf))
}

/// Reads the headers of the boxes packed between `start` and `end`.
fn children(file: &mut File, start: u64, end: u64) -> io::Result<Vec<BoxRef>> {
    let mut boxes = Vec::new();
    let mut offset = start;
    while boxes.len() < MAX_BOXES {
        let Some(found) = header_at(file, offset, end)? else { break };
        offset = found.end;
        boxes.push(found);
    }
    Ok(boxes)
}

fn header_at(file: &mut File, offset: u64, end: u64) -> io::Result<Option<BoxRef>> {
    let room = end.saturating_sub(offset);
    let avail = room.min(16) as usize;
    if avail < 8 {
        return Ok(None);
    }
    let mut buf = [0; 16];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut buf[..avail])?;
    Ok(parse_header(&buf[..avail], room).map(|(kind, header, size)| BoxRef {
        kind,
        payload: offset + header,
        end: offset + size,
    }))
}

/// Parses a box header with `room` bytes left in its parent.
/// Returns (type, header length, total size).
fn parse_header(buf: &[u8], room: u64) -> Option<([u8; 4], u64, u64)> {
    let kind = buf.get(4..8)?.try_into().ok()?;
    let (header, size) = match be32(buf, 0)? {
        0 => (8, room),
        1 => (16, be64(buf, 8)?),
        n => (8, u64::from(n)),
    };
    (size >= header && size <= room).then_some((kind, header, size))
}

/// Iterates the boxes packed in `buf`, yielding (type, payload).
fn boxes(buf: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut rest = buf;
    std::iter::from_fn(move || {
        let (kind, header, size) = parse_header(rest, rest.len() as u64)?;
        let (this, next) = rest.split_at(size as usize);
        rest = next;
        Some((kind, &this[header as usize..]))
    })
}

/// Finds a `keys` box directly followed by its `ilst` anywhere in `buf`, last one first.
fn gamma_in_bytes(buf: &[u8]) -> Option<String> {
    buf.windows(4)
        .enumerate()
        .rev()
        .filter(|&(at, window)| window == b"keys" && at >= 4)
        .find_map(|(at, _)| {
            let mut pair = boxes(&buf[at - 4..]);
            match (pair.next(), pair.next()) {
                (Some((k, keys)), Some((i, ilst))) if &k == b"keys" && &i == b"ilst" => lookup(keys, ilst),
                _ => None,
            }
        })
}

/// Returns the `ilst` value stored under `GAMMA_KEY`.
fn lookup(keys: &[u8], ilst: &[u8]) -> Option<String> {
    let index = key_index(keys)?;
    let (_, item) = boxes(ilst).find(|(kind, _)| u32::from_be_bytes(*kind) == index)?;
    item_value(item)
}

/// The 1-based index of `GAMMA_KEY` in a `keys` payload: `ilst` items are boxes typed by it.
fn key_index(keys: &[u8]) -> Option<u32> {
    let position = key_names(keys).position(|name| name == GAMMA_KEY.as_bytes())?;
    u32::try_from(position + 1).ok()
}

/// The value of an `ilst` item, whose `data` payload is a 4-byte type, a 4-byte locale, then the value.
fn item_value(item: &[u8]) -> Option<String> {
    let (_, data) = boxes(item).find(|(kind, _)| kind == b"data")?;
    let value = String::from_utf8_lossy(data.get(8..)?);
    Some(value.trim_end_matches('\0').to_owned())
}

/// A `keys` payload is version/flags, a count, then (size, namespace, name) entries,
/// which are shaped exactly like boxes.
fn key_names(keys: &[u8]) -> impl Iterator<Item = &[u8]> {
    let count = be32(keys, 4).unwrap_or(0) as usize;
    boxes(keys.get(8..).unwrap_or_default())
        .take(count)
        .map(|(_, name)| name)
}

fn be32(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(buf.get(at..at + 4)?.try_into().ok()?))
}

fn be64(buf: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(buf.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    pub fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(payload);
        out
    }

    /// A `meta` box laid out the way DJI writes it, with a `thumbnail`-byte item after the profile.
    fn dji_meta(gamma: &str, iso: bool, thumbnail: usize) -> Vec<u8> {
        let names = ["com.dji.camera.CameraModel", GAMMA_KEY, "com.dji.camera.LensType", "com.dji.thumbnail"];
        let mut keys = vec![0, 0, 0, 0];
        keys.extend((names.len() as u32).to_be_bytes());
        for name in names {
            keys.extend(atom(b"mdta", name.as_bytes()));
        }
        let item = |index: u32, value: &[u8]| {
            let mut data = 1u32.to_be_bytes().to_vec();
            data.extend([0; 4]);
            data.extend(value);
            atom(&index.to_be_bytes(), &atom(b"data", &data))
        };
        let mut ilst = [item(1, b"PP-041"), item(2, gamma.as_bytes()), item(3, b"20mm F2")].concat();
        if thumbnail > 0 {
            ilst.extend(item(4, &vec![0xFF; thumbnail]));
        }
        let mut payload = if iso { vec![0; 4] } else { vec![] };
        payload.extend(atom(b"hdlr", &[0; 24]));
        payload.extend(atom(b"keys", &keys));
        payload.extend(atom(b"ilst", &ilst));
        atom(b"meta", &payload)
    }

    /// A small MP4 shaped like a DJI clip: thumbnails in `udta/meta`, profile in `moov/meta`.
    pub fn fake_clip(gamma: Option<&str>) -> Vec<u8> {
        build(gamma, false, false, 0)
    }

    fn build(gamma: Option<&str>, moov_first: bool, iso_meta: bool, thumbnail: usize) -> Vec<u8> {
        let ftyp = atom(b"ftyp", b"isom\0\0\0\0isom");
        let mdat = atom(b"mdat", &vec![0xAB; 5000]);
        let thumbnails = atom(b"meta", &[atom(b"hdlr", &[0; 24]), atom(b"ilst", &vec![0xFF; 80_000])].concat());
        let mut moov = [atom(b"mvhd", &[0; 100]), atom(b"trak", &[0; 3000]), atom(b"udta", &thumbnails)].concat();
        if let Some(gamma) = gamma {
            moov.extend(dji_meta(gamma, iso_meta, thumbnail));
        }
        let moov = atom(b"moov", &moov);
        if moov_first { [ftyp, moov, mdat].concat() } else { [ftyp, mdat, moov].concat() }
    }

    pub fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("d-sep-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn probe_bytes(name: &str, bytes: &[u8]) -> Option<String> {
        let dir = temp_dir(name);
        let path = dir.join("clip.mp4");
        std::fs::write(&path, bytes).unwrap();
        let gamma = read_gamma(&path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        gamma
    }

    #[test]
    fn reads_log_profiles() {
        assert_eq!(probe_bytes("dlog", &fake_clip(Some("D-Log"))).as_deref(), Some("D-Log"));
        assert_eq!(probe_bytes("dlog2", &fake_clip(Some("D-Log2"))).as_deref(), Some("D-Log2"));
        assert_eq!(probe_bytes("dlogm", &fake_clip(Some("D-Log M"))).as_deref(), Some("D-Log M"));
    }

    #[test]
    fn handles_moov_first_and_iso_meta() {
        assert_eq!(probe_bytes("faststart", &build(Some("D-Log2"), true, false, 0)).as_deref(), Some("D-Log2"));
        assert_eq!(probe_bytes("iso", &build(Some("D-Log"), false, true, 0)).as_deref(), Some("D-Log"));
    }

    #[test]
    fn reads_a_big_meta() {
        // A 200 KB thumbnail next to the profile makes `moov/meta` far bigger than DJI's ~600 bytes.
        let clip = build(Some("D-Log M"), false, false, 200_000);
        assert_eq!(probe_bytes("bigmeta", &clip).as_deref(), Some("D-Log M"));
        let clip = build(Some("D-Log"), true, true, 200_000);
        assert_eq!(probe_bytes("bigmetaiso", &clip).as_deref(), Some("D-Log"));
    }

    #[test]
    fn handles_64_bit_box_sizes() {
        let clip = fake_clip(Some("D-Log2"));
        // Rewrite `mdat` (after the 20-byte ftyp) with a 16-byte "largesize" header.
        let (head, rest) = clip.split_at(20);
        let size = u32::from_be_bytes(rest[..4].try_into().unwrap()) as u64;
        let mut wide = head.to_vec();
        wide.extend(1u32.to_be_bytes());
        wide.extend(b"mdat");
        wide.extend((size + 8).to_be_bytes());
        wide.extend(&rest[8..]);
        assert_eq!(probe_bytes("wide", &wide).as_deref(), Some("D-Log2"));
    }

    #[test]
    fn falls_back_to_tail_scan_when_box_chain_is_broken() {
        let mut clip = fake_clip(Some("D-Log"));
        clip[20..24].copy_from_slice(&7u32.to_be_bytes()); // mangle the `mdat` size
        assert_eq!(probe_bytes("broken", &clip).as_deref(), Some("D-Log"));
    }

    #[test]
    fn missing_metadata_is_none() {
        assert_eq!(probe_bytes("plain", &fake_clip(None)), None);
        assert_eq!(probe_bytes("junk", b"not a video at all"), None);
        assert_eq!(probe_bytes("empty", b""), None);
    }

    #[test]
    fn normalizes_profile_names() {
        assert_eq!(Profile::from_gamma("D-Log"), Profile::DLog);
        assert_eq!(Profile::from_gamma(" D-Log2\0"), Profile::DLog2);
        assert_eq!(Profile::from_gamma("d-log 2"), Profile::DLog2);
        assert_eq!(Profile::from_gamma("D-Log M"), Profile::DLogM);
        assert_eq!(Profile::from_gamma("D-LogM"), Profile::DLogM);
        assert_eq!(Profile::from_gamma("Rec.709"), Profile::Other("Rec.709".into()));
        assert_eq!(Profile::from_gamma("Rec.2100 HLG"), Profile::Other("Rec.2100 HLG".into()));
        assert_eq!(Profile::from_gamma("  "), Profile::Unknown);
        assert_eq!(Profile::from_gamma("Rec\u{7}.709"), Profile::Other("Rec.709".into()));
        let long = Profile::from_gamma(&"x".repeat(5000));
        assert_eq!(long, Profile::Other(format!("{}…", "x".repeat(MAX_LABEL_CHARS))));
    }

    /// The shape `Profile` in src/lib/api.ts expects: `{ kind }`, plus `label` for other profiles.
    #[test]
    fn serializes_profiles_for_the_ui() {
        let json = |profile: Profile| serde_json::to_value(profile).unwrap();
        assert_eq!(json(Profile::DLog), serde_json::json!({ "kind": "dlog" }));
        assert_eq!(json(Profile::DLog2), serde_json::json!({ "kind": "dlog2" }));
        assert_eq!(json(Profile::DLogM), serde_json::json!({ "kind": "dlogm" }));
        assert_eq!(json(Profile::Unknown), serde_json::json!({ "kind": "unknown" }));
        assert_eq!(json(Profile::Other("Rec.709".into())), serde_json::json!({ "kind": "other", "label": "Rec.709" }));
    }

    /// Prints the profile of real footage, subfolders included:
    /// `D_SEP_SAMPLES=<dir> cargo test -- --ignored --nocapture`. A relative `<dir>` is taken from the project root.
    #[test]
    #[ignore]
    fn real_samples() {
        let dir = PathBuf::from(std::env::var("D_SEP_SAMPLES").expect("set D_SEP_SAMPLES"));
        let mut dirs = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(dir)];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    println!("{} -> {:?}", path.display(), read_gamma(&path));
                }
            }
        }
    }
}
