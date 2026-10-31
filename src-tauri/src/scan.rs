//! Finds the clips in a folder, their log profile, and the sidecar files that travel with them.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use serde::Serialize;

use crate::probe::{self, Profile};

pub const DLOG_DIR: &str = "D-Log";
pub const DLOG2_DIR: &str = "D-Log 2";
pub const DLOGM_DIR: &str = "D-Log M";
const OUTPUT_DIRS: [&str; 3] = [DLOG_DIR, DLOG2_DIR, DLOGM_DIR];

const VIDEO_EXTENSIONS: [&str; 2] = ["mp4", "mov"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub path: PathBuf,
    /// Path relative to the scanned folder, for display.
    pub name: String,
    pub size: u64,
    pub profile: Profile,
    /// Files next to the clip with the same name, e.g. `.LRF` proxies and `.SRT` telemetry.
    pub sidecars: Vec<Sidecar>,
    /// Set when the file could not be read.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sidecar {
    pub path: PathBuf,
    pub size: u64,
}

/// How far a scan has got.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    /// Videos found so far.
    pub found: usize,
    /// How many of them have been read; 0 while folders are still being listed.
    pub probed: usize,
}

/// Lets a scan report how far it's got, and stops it when `cancelled` turns true.
/// Both are called from several threads at once.
pub struct Watch<'a> {
    pub report: &'a (dyn Fn(ScanProgress) + Sync),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

/// A video `collect` found, with its sidecars.
struct Found {
    path: PathBuf,
    size: u64,
    sidecars: Vec<Sidecar>,
    /// Kept by a cloud drive like OneDrive online only: reading its profile would download all of it.
    online_only: bool,
}

/// `scan_watched` without progress or cancel, for tests.
#[cfg(test)]
pub fn scan(root: &Path, recursive: bool) -> io::Result<Vec<Clip>> {
    let watch = Watch { report: &|_| {}, cancelled: &|| false };
    Ok(scan_watched(root, recursive, &watch)?.unwrap_or_default())
}

/// Lists the clips in `root`, with their profiles and sidecars, reporting progress. None when cancelled.
pub fn scan_watched(root: &Path, recursive: bool, watch: &Watch) -> io::Result<Option<Vec<Clip>>> {
    let mut found = Vec::new();
    // When the folder is itself D-Log, D-Log 2 or D-Log M, the output folders are beside it, not in it.
    let outputs_inside = !root.file_name().and_then(|name| name.to_str()).is_some_and(is_output_dir);
    collect(root, recursive, outputs_inside, &mut found, watch)?;
    let Some(profiles) = detect_all(&found, watch) else { return Ok(None) };

    let mut clips: Vec<Clip> = found
        .into_iter()
        .zip(profiles)
        .map(|(Found { path, size, sidecars, .. }, detected)| {
            let (profile, error) = match detected {
                Ok(profile) => (profile, None),
                Err(e) => (Profile::Unknown, Some(crate::describe(e))),
            };
            let name = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            Clip { path, name, size, profile, sidecars, error }
        })
        .collect();
    // Folder by folder, each folder's own clips before its subfolders', so clips from a subfolder don't mix in with
    // those of the folder above it.
    clips.sort_by_cached_key(|clip| {
        let name = Path::new(&clip.name);
        let lower = |part: &std::ffi::OsStr| part.to_string_lossy().to_lowercase();
        let folders: Vec<String> = name.parent().into_iter().flat_map(Path::iter).map(lower).collect();
        (folders, name.file_name().map(lower).unwrap_or_default())
    });
    Ok(Some(clips))
}

/// Lists the videos in `dir`, and in its subfolders when `recursive`, with their sidecars.
/// Sidecars are matched one folder at a time, so the other files of a big tree are never all held at once.
fn collect(dir: &Path, recursive: bool, skip_outputs: bool, out: &mut Vec<Found>, watch: &Watch) -> io::Result<()> {
    let (mut videos, mut others, mut folders) = (Vec::new(), Vec::new(), Vec::new());
    for entry in fs::read_dir(dir)?.flatten() {
        // A name that isn't valid Unicode can't be sent to the UI, and would fail the whole scan.
        let Ok(name) = entry.file_name().into_string() else { continue };
        // Hidden files, macOS "._" resource forks, $RECYCLE.BIN and friends.
        if name.starts_with('.') || name.starts_with('$') {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_file() {
            let Ok(meta) = entry.metadata() else { continue };
            let path = entry.path();
            if is_video(&path) {
                videos.push((path, meta.len(), online_only(&meta)));
            } else {
                others.push((path, meta.len()));
            }
        } else if kind.is_dir() && recursive && !skip_dir(&name, skip_outputs) {
            folders.push(entry.path());
        }
    }

    let mut sidecars: HashMap<String, Vec<Sidecar>> = HashMap::new();
    for (path, size) in others {
        sidecars.entry(stem(&path)).or_default().push(Sidecar { path, size });
    }
    // Sidecars an .mp4 and a .mov share are the .mp4's: DJI cameras write their .LRF and .SRT files next to .MP4s, so
    // a .mov of the same name is more likely an export.
    videos.sort_by_key(|(path, ..)| !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mp4")));
    for (path, size, online_only) in videos {
        let mut own = sidecars.remove(&stem(&path)).unwrap_or_default();
        own.sort_by(|a, b| a.path.cmp(&b.path));
        out.push(Found { path, size, sidecars: own, online_only });
    }
    (watch.report)(ScanProgress { found: out.len(), probed: 0 });

    for folder in folders {
        if (watch.cancelled)() {
            break;
        }
        // An unreadable subfolder shouldn't fail the whole scan.
        let _ = collect(&folder, true, false, out, watch);
    }
    Ok(())
}

/// Our own output folders are never rescanned.
fn skip_dir(name: &str, skip_outputs: bool) -> bool {
    name.eq_ignore_ascii_case("System Volume Information") || (skip_outputs && is_output_dir(name))
}

/// Whether `name` is one of the D-Log, D-Log 2 and D-Log M folders.
pub fn is_output_dir(name: &str) -> bool {
    OUTPUT_DIRS.iter().any(|dir| name.eq_ignore_ascii_case(dir))
}

/// Whether a cloud drive keeps the file's data online only, or it's offline: reading it would download it.
#[cfg(windows)]
fn online_only(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS};
    meta.file_attributes() & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(windows))]
fn online_only(_meta: &fs::Metadata) -> bool {
    false
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| VIDEO_EXTENSIONS.iter().any(|v| ext.eq_ignore_ascii_case(v)))
}

/// A file's name without its extension, lowercased: what a clip and its sidecars share.
fn stem(path: &Path) -> String {
    path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase()
}

/// Probes files on up to 8 threads, each taking the next file when it's done with one, so a slow file holds up
/// only its own thread. Results stay in input order. None when cancelled.
fn detect_all(found: &[Found], watch: &Watch) -> Option<Vec<io::Result<Profile>>> {
    let workers = thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let (next, probed) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let parts: Vec<Vec<(usize, io::Result<Profile>)>> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers.min(found.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    while !(watch.cancelled)() {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(file) = found.get(at) else { break };
                        let detected =
                            if file.online_only { Err(not_on_this_device()) } else { probe::detect(&file.path) };
                        mine.push((at, detected));
                        let probed = probed.fetch_add(1, Ordering::Relaxed) + 1;
                        (watch.report)(ScanProgress { found: found.len(), probed });
                    }
                    mine
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("probe thread panicked")).collect()
    });
    if (watch.cancelled)() {
        return None;
    }
    let mut results: Vec<Option<io::Result<Profile>>> = found.iter().map(|_| None).collect();
    for (at, result) in parts.into_iter().flatten() {
        results[at] = Some(result);
    }
    results.into_iter().collect()
}

fn not_on_this_device() -> io::Error {
    io::Error::other("It's only in the cloud. Make it available on this device, then scan again.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::tests::{fake_clip, temp_dir};

    #[test]
    fn finds_clips_profiles_and_sidecars() {
        let root = temp_dir("scan");
        fs::write(root.join("A_D.MP4"), fake_clip(Some("D-Log"))).unwrap();
        fs::write(root.join("A_D.LRF"), b"proxy").unwrap();
        fs::write(root.join("B_D.mp4"), fake_clip(Some("D-Log2"))).unwrap();
        fs::write(root.join("b_d.SRT"), b"telemetry").unwrap();
        fs::write(root.join("C.MOV"), fake_clip(Some("D-Log M"))).unwrap();
        fs::write(root.join("D.mp4"), b"not a DJI clip").unwrap();
        fs::write(root.join("H.MP4"), fake_clip(Some("Rec.2100 HLG"))).unwrap();
        fs::write(root.join("._A_D.MP4"), b"macOS junk").unwrap();
        fs::write(root.join("notes.txt"), b"unrelated").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("E.MP4"), fake_clip(Some("D-Log"))).unwrap();
        fs::create_dir_all(root.join("D-Log 2")).unwrap();
        fs::write(root.join("D-Log 2").join("F.MP4"), fake_clip(Some("D-Log2"))).unwrap();
        fs::create_dir_all(root.join("D-Log M")).unwrap();
        fs::write(root.join("D-Log M").join("G.MP4"), fake_clip(Some("D-Log M"))).unwrap();

        let clips = scan(&root, false).unwrap();
        let summary: Vec<_> = clips.iter().map(|c| (c.name.as_str(), &c.profile, c.sidecars.len())).collect();
        assert_eq!(
            summary,
            [
                ("A_D.MP4", &Profile::DLog, 1),
                ("B_D.mp4", &Profile::DLog2, 1),
                ("C.MOV", &Profile::DLogM, 0),
                ("D.mp4", &Profile::Unknown, 0),
                ("H.MP4", &Profile::Other("Rec.2100 HLG".into()), 0),
            ]
        );

        let names: Vec<_> = scan(&root, true).unwrap().into_iter().map(|c| c.name).collect();
        let sub = format!("sub{}E.MP4", std::path::MAIN_SEPARATOR);
        assert_eq!(names, ["A_D.MP4", "B_D.mp4", "C.MOV", "D.mp4", "H.MP4", &sub]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn skips_names_that_are_not_unicode() {
        use std::os::windows::ffi::OsStringExt;
        let root = temp_dir("unicode");
        // A lone surrogate: a legal NTFS name with no UTF-8 form.
        let name = std::ffi::OsString::from_wide(&[0xD800, 'X' as u16, '.' as u16, 'M' as u16, 'P' as u16, '4' as u16]);
        fs::write(root.join(name), fake_clip(Some("D-Log"))).unwrap();
        fs::write(root.join("A.MP4"), fake_clip(Some("D-Log"))).unwrap();
        let names: Vec<_> = scan(&root, true).unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["A.MP4"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gives_sidecars_an_mp4_and_a_mov_share_to_the_mp4() {
        let root = temp_dir("sharedname");
        fs::write(root.join("A.MOV"), fake_clip(Some("D-Log"))).unwrap();
        fs::write(root.join("A.MP4"), fake_clip(Some("D-Log"))).unwrap();
        fs::write(root.join("A.LRF"), b"proxy").unwrap();
        let clips = scan(&root, false).unwrap();
        let sidecars: Vec<_> = clips.iter().map(|c| (c.name.as_str(), c.sidecars.len())).collect();
        assert_eq!(sidecars, [("A.MOV", 0), ("A.MP4", 1)]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lists_a_folders_own_clips_before_its_subfolders() {
        let root = temp_dir("order");
        // "A B" sorts before "A\C" as text, but it isn't inside A.
        let files: Vec<PathBuf> =
            [&["Z.MP4"][..], &["A", "B.MP4"], &["A", "C", "D.MP4"], &["A B", "E.MP4"]].map(|p| p.iter().collect()).into();
        for file in &files {
            fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            fs::write(root.join(file), fake_clip(Some("D-Log"))).unwrap();
        }
        let names: Vec<_> = scan(&root, true).unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, files.iter().map(|f| f.to_string_lossy()).collect::<Vec<_>>());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn leaves_files_kept_only_in_the_cloud_unread() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_OFFLINE, SetFileAttributesW};
        let root = temp_dir("cloud");
        let clip = root.join("A.MP4");
        fs::write(&clip, fake_clip(Some("D-Log"))).unwrap();
        let wide: Vec<u16> = clip.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: the path is NUL-terminated and lives across the call.
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_OFFLINE) }, 0);
        let clips = scan(&root, false).unwrap();
        assert_eq!(clips[0].profile, Profile::Unknown);
        assert!(clips[0].error.as_deref().unwrap().contains("only in the cloud"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_progress_and_stops_when_cancelled() {
        let root = temp_dir("scanwatch");
        for name in ["A.MP4", "B.MP4", "C.MP4"] {
            fs::write(root.join(name), fake_clip(Some("D-Log"))).unwrap();
        }
        // Sidecars only match within their own folder.
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("A.LRF"), b"another clip's proxy").unwrap();

        let (found, probed) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let report = |p: ScanProgress| {
            found.fetch_max(p.found, Ordering::Relaxed);
            probed.fetch_max(p.probed, Ordering::Relaxed);
        };
        let watch = Watch { report: &report, cancelled: &|| false };
        let clips = scan_watched(&root, true, &watch).unwrap().unwrap();
        assert_eq!(clips.iter().map(|c| c.sidecars.len()).sum::<usize>(), 0);
        assert_eq!((found.into_inner(), probed.into_inner()), (3, 3));

        let watch = Watch { report: &|_| {}, cancelled: &|| true };
        assert!(scan_watched(&root, true, &watch).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_folder_is_an_error() {
        assert!(scan(Path::new("Z:\\definitely\\not\\here"), false).is_err());
    }
}
