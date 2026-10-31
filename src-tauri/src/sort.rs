//! Copies or moves D-Log / D-Log 2 / D-Log M clips, with their sidecars, into their folders,
//! renames them with a prefix, or both, and undoes a move.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::probe::Profile;
use crate::scan::{Clip, DLOG_DIR, DLOG2_DIR, DLOGM_DIR, Sidecar, is_output_dir};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Into the `D-Log` / `D-Log 2` / `D-Log M` folders, names unchanged.
    Folders,
    /// A prefix on the name, in the clip's own folder.
    Rename,
    /// Into the folders, with the prefix.
    Both,
}

/// Where the profile folders go for clips in subfolders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// One set of D-Log, D-Log 2 and D-Log M folders at the top.
    Gather,
    /// A set inside each subfolder.
    Each,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Prefixes {
    pub dlog: String,
    pub dlog2: String,
    pub dlogm: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub mode: Mode,
    pub action: Action,
    pub sidecars: bool,
    /// Ignored for `Action::Folders`.
    pub prefixes: Prefixes,
    /// Where the clips go, subfolders kept; None for the scanned folder itself.
    pub destination: Option<PathBuf>,
    /// Ignored for `Action::Rename`.
    pub layout: Layout,
    /// Read every copy back from the disk and compare it with its original.
    pub verify: bool,
}

/// The same as the UI's `MAX_PREFIX`.
const MAX_PREFIX_CHARS: usize = 16;

impl Options {
    /// Rejects a prefix that isn't a plain piece of a file name, or a destination that's gone,
    /// before anything is touched.
    pub fn check(&self) -> Result<(), String> {
        if self.destination.as_ref().is_some_and(|dir| !dir.is_absolute() || !dir.is_dir()) {
            return Err("The destination folder no longer exists.".into());
        }
        if self.action == Action::Folders {
            return Ok(());
        }
        let Prefixes { dlog, dlog2, dlogm } = &self.prefixes;
        for (profile, prefix) in [("D-Log", dlog), ("D-Log 2", dlog2), ("D-Log M", dlogm)] {
            check_prefix(prefix).map_err(|problem| format!("The {profile} prefix {problem}."))?;
        }
        Ok(())
    }
}

fn check_prefix(prefix: &str) -> Result<(), &'static str> {
    if prefix.trim().is_empty() {
        Err("is empty")
    } else if prefix.chars().count() > MAX_PREFIX_CHARS {
        Err("is too long")
    } else if prefix.chars().any(char::is_control) {
        Err("can't contain control characters")
    } else if prefix.chars().any(|c| r#"<>:"/\|?*"#.contains(c)) {
        Err(r#"can't contain \ / : * ? " < > |"#)
    } else if prefix.starts_with(['.', '$']) || prefix.starts_with(char::is_whitespace) {
        // The scan skips names starting with "." or "$", so the clips would vanish from the next one.
        Err("can't start with a dot, $ or a space")
    } else if prefix.split_once('.').is_some_and(|(name, _)| is_device_name(name)) {
        // "NUL.DJI_0001.MP4" is the NUL device to Explorer and most programs, which couldn't open the clip.
        Err("can't start with a device name like CON or NUL")
    } else {
        Ok(())
    }
}

/// CON, PRN, AUX, NUL, COM0–COM9 and LPT0–LPT9, which Windows keeps for devices even with an extension after them.
fn is_device_name(name: &str) -> bool {
    let name = name.trim_end().to_ascii_uppercase();
    ["CON", "PRN", "AUX", "NUL"].contains(&name.as_str())
        || (name.len() == 4 && (name.starts_with("COM") || name.starts_with("LPT")) && name.ends_with(|c: char| c.is_ascii_digit()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// 1-based position of the clip being processed.
    pub index: usize,
    pub count: usize,
    pub file: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Reading a copy back to compare it with its original, rather than copying.
    pub verifying: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub dlog: usize,
    pub dlog2: usize,
    pub dlogm: usize,
    pub sidecars: usize,
    /// Clips already in their folder (same size and last-write time), left untouched.
    pub existing: usize,
    pub failed: Vec<Failure>,
    pub cancelled: bool,
    /// Where the clips went: the destination, or the scanned folder.
    pub folder: PathBuf,
    /// For each profile, the folder its clips are in now, when that's a single folder.
    pub folders: ProfileFolders,
    /// Whether files were moved, so `undo` has something to put back.
    pub undoable: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct ProfileFolders {
    pub dlog: Option<PathBuf>,
    pub dlog2: Option<PathBuf>,
    pub dlogm: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub file: String,
    pub reason: String,
}

/// What `undo` did.
#[derive(Debug, Default, Serialize)]
pub struct Undone {
    /// Files moved back.
    pub restored: usize,
    /// Files still to move back, so undo can be tried again.
    pub remaining: usize,
    pub failed: Vec<Failure>,
    pub cancelled: bool,
}

/// What a move did, so `undo` can put it back.
#[derive(Debug, Default)]
pub struct Journal {
    /// Files moved, oldest first: (from, to).
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// Folders the sort made, parents first.
    pub created: Vec<PathBuf>,
    /// The sort verified its copies, so moving them back across drives does too.
    pub verify: bool,
}

pub struct Outcome {
    pub summary: Summary,
    pub journal: Journal,
}

/// Sidecars are small, so once a clip is placed its sidecars always follow, even mid-cancel.
static NEVER: AtomicBool = AtomicBool::new(false);

/// FAT keeps file times to 2 s, so a copy's time can be rounded.
const TIME_SLACK: Duration = Duration::from_secs(2);

/// Where one log clip goes, and the prefix it gets ("" to keep its name).
struct Job<'a> {
    clip: &'a Clip,
    dir: PathBuf,
    prefix: &'a str,
}

impl<'a> Job<'a> {
    /// None for clips that stay where they are.
    fn new(root: &Path, clip: &'a Clip, options: &'a Options) -> Option<Self> {
        let (folder, prefix) = match clip.profile {
            Profile::DLog => (DLOG_DIR, options.prefixes.dlog.as_str()),
            Profile::DLog2 => (DLOG2_DIR, options.prefixes.dlog2.as_str()),
            Profile::DLogM => (DLOGM_DIR, options.prefixes.dlogm.as_str()),
            Profile::Other(_) | Profile::Unknown => return None,
        };
        let base = options.destination.as_deref().unwrap_or(root);
        // The clip's own folder, as it is under `base`. A clip already inside a destination that's inside the scanned
        // folder is where an earlier sort put it: it keeps its place there, rather than going into another copy of it.
        let parent = clip.path.parent()?;
        let sub = under(base, root).and(under(parent, base)).or_else(|| under(parent, root))?;
        let own = if sub.as_os_str().is_empty() { base.to_path_buf() } else { base.join(sub) };
        let dir = match (options.action, options.layout) {
            (Action::Rename, _) => own,
            (_, Layout::Each) => output_dir(&own, folder),
            (_, Layout::Gather) => output_dir(base, folder),
        };
        // A clip that already starts with the prefix isn't prefixed twice.
        let renamed = options.action != Action::Folders && !has_prefix(&clip.path, prefix);
        Some(Job { clip, dir, prefix: if renamed { prefix } else { "" } })
    }
}

/// The profile's folder: inside `dir`, or beside it when `dir` is one of the folders,
/// so sorting the D-Log folder itself doesn't make `D-Log\D-Log`.
fn output_dir(dir: &Path, folder: &str) -> PathBuf {
    match dir.file_name().and_then(OsStr::to_str) {
        Some(name) if name.eq_ignore_ascii_case(folder) => dir.to_path_buf(),
        Some(name) if is_output_dir(name) => dir.with_file_name(folder),
        _ => dir.join(folder),
    }
}

/// The part of `path` under `dir`, ignoring case as Windows does: "" for `dir` itself, None for a path outside it.
/// The scanned folder and the destination are picked separately, so they can spell the same folder differently.
fn under<'a>(path: &'a Path, dir: &Path) -> Option<&'a Path> {
    let lower = |part: std::path::Component| part.as_os_str().to_string_lossy().to_lowercase();
    let mut rest = path.components();
    for part in dir.components() {
        if lower(rest.next()?) != lower(part) {
            return None;
        }
    }
    Some(rest.as_path())
}

pub fn run(
    root: &Path,
    clips: &[Clip],
    options: &Options,
    cancel: &AtomicBool,
    mut report: impl FnMut(&Progress),
) -> Outcome {
    let with_sidecars = options.sidecars;
    // Verifying reads every copy back, as much work again as copying it.
    let passes = if options.verify { 2 } else { 1 };
    let jobs: Vec<Job> = clips.iter().filter_map(|clip| Job::new(root, clip, options)).collect();
    // Clips already in place are left alone, so the progress counts only the others, as the UI's button does.
    let (already, jobs): (Vec<Job>, Vec<Job>) = jobs.into_iter().partition(|job| is_placed(job, with_sidecars));
    let mut progress = Progress {
        index: 0,
        count: jobs.len(),
        file: String::new(),
        bytes_done: 0,
        bytes_total: jobs.iter().map(|job| job_bytes(job.clip, with_sidecars) * passes).sum(),
        verifying: false,
    };
    let mut summary = Summary { existing: already.len(), ..Summary::default() };
    let mut journal = Journal { verify: options.verify, ..Journal::default() };
    // Per profile: None before its first clip, then Some(its folder), or Some(None) once there's a second.
    let mut folders: [Option<Option<&Path>>; 3] = [None; 3];
    for job in &already {
        note_folder(&mut folders, job);
    }

    for job in &jobs {
        let clip = job.clip;
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        progress.index += 1;
        progress.file = clip.name.clone();
        progress.verifying = false;
        report(&progress);

        let start = progress.bytes_done;
        let sidecars = sidecars_of(clip, with_sidecars);
        let mut on_bytes = |done| {
            progress.bytes_done = start + done;
            // The clip's bytes come twice when verifying: copied, then read back.
            progress.verifying = passes == 2 && (clip.size..2 * clip.size).contains(&done);
            report(&progress);
        };
        let placed = place(job, sidecars, options, passes, cancel, &mut summary, &mut journal, &mut on_bytes);
        if placed.is_ok() {
            note_folder(&mut folders, job);
        }
        match placed {
            Ok(true) => match clip.profile {
                Profile::DLog => summary.dlog += 1,
                Profile::DLog2 => summary.dlog2 += 1,
                Profile::DLogM => summary.dlogm += 1,
                // Never queued.
                Profile::Other(_) | Profile::Unknown => {}
            },
            Ok(false) => summary.existing += 1,
            Err(_) if cancel.load(Ordering::Relaxed) => {
                summary.cancelled = true;
                break;
            }
            Err(e) => summary.failed.push(Failure { file: clip.name.clone(), reason: crate::describe(e) }),
        }
        progress.bytes_done = start + job_bytes(clip, with_sidecars) * passes;
    }
    progress.verifying = false;
    report(&progress);

    // Folders made for clips that then failed, or were cancelled, are taken away again. remove_dir leaves the
    // ones that aren't empty. Deepest first.
    for dir in journal.created.iter().rev() {
        let _ = fs::remove_dir(dir);
    }
    journal.created.retain(|dir| dir.exists());

    let one = |slot: Option<Option<&Path>>| slot.flatten().map(Path::to_path_buf);
    summary.folders = ProfileFolders { dlog: one(folders[0]), dlog2: one(folders[1]), dlogm: one(folders[2]) };
    summary.folder = options.destination.clone().unwrap_or_else(|| root.to_path_buf());
    summary.undoable = !journal.moved.is_empty();
    Outcome { summary, journal }
}

/// Notes the folder a job's clip is in now, in its profile's slot of `run`'s `folders`.
fn note_folder<'a>(folders: &mut [Option<Option<&'a Path>>; 3], job: &'a Job) {
    let slot = &mut folders[match job.clip.profile {
        Profile::DLog => 0,
        Profile::DLog2 => 1,
        _ => 2,
    }];
    *slot = match *slot {
        None => Some(Some(&job.dir)),
        Some(Some(seen)) if seen != job.dir => Some(None),
        same => same,
    };
}

fn sidecars_of(clip: &Clip, include: bool) -> &[Sidecar] {
    if include { &clip.sidecars } else { &[] }
}

fn job_bytes(clip: &Clip, with_sidecars: bool) -> u64 {
    clip.size + sidecars_of(clip, with_sidecars).iter().map(|s| s.size).sum::<u64>()
}

/// Whether the file's name already starts with `prefix`, ignoring case as Windows does.
fn has_prefix(path: &Path, prefix: &str) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    name.starts_with(&prefix.to_lowercase())
}

/// Places one clip and its sidecars in the job's folder, with its prefix in front of their names.
/// Returns false if it was already there.
#[allow(clippy::too_many_arguments)]
fn place(
    job: &Job,
    sidecars: &[Sidecar],
    options: &Options,
    passes: u64,
    cancel: &AtomicBool,
    summary: &mut Summary,
    journal: &mut Journal,
    on_bytes: &mut dyn FnMut(u64),
) -> io::Result<bool> {
    let Job { clip, ref dir, prefix } = *job;
    let dir = dir.as_path();
    // Renaming in place a clip that already has its prefix: it's already where it belongs.
    if prefix.is_empty() && clip.path.parent() == Some(dir) {
        return Ok(false);
    }
    make_dir(dir, &mut journal.created)?;
    let original = clip.path.file_stem().unwrap_or_default();
    let (stem, there) = stem_in(clip, dir, prefix, sidecars);
    let (mode, verify) = (options.mode, options.verify);
    if !there {
        let target = dir.join(target_name(&clip.path, prefix, &stem, original));
        transfer(&clip.path, &target, mode, verify, cancel, on_bytes)?;
        if mode == Mode::Move {
            journal.moved.push((clip.path.clone(), target));
        }
    }

    // A clip that was already there still gets the sidecars it lacks, e.g. after a run without them. The clip
    // itself stays where it is, so even in a move they're copied, not taken away from it.
    let sidecar_mode = if there { Mode::Copy } else { mode };
    let mut offset = clip.size * passes;
    for sidecar in sidecars {
        let target = dir.join(target_name(&sidecar.path, prefix, &stem, original));
        if !there || !target.exists() {
            match transfer(&sidecar.path, &target, sidecar_mode, verify, &NEVER, &mut |done| on_bytes(offset + done)) {
                Ok(()) => {
                    summary.sidecars += 1;
                    if sidecar_mode == Mode::Move {
                        journal.moved.push((sidecar.path.clone(), target));
                    }
                }
                Err(e) => summary.failed.push(Failure { file: sidecar_name(clip, sidecar), reason: crate::describe(e) }),
            }
        }
        offset += sidecar.size * passes;
    }
    Ok(!there)
}

/// The clips a run with `options` would leave as they are: already where it would put them, with the sidecars it
/// would bring. The same checks as `place`, without touching anything.
pub fn in_place(root: &Path, clips: &[Clip], options: &Options) -> Vec<PathBuf> {
    let placed = |job: &Job| is_placed(job, options.sidecars);
    clips.iter().filter_map(|clip| Job::new(root, clip, options)).filter(placed).map(|job| job.clip.path.clone()).collect()
}

/// Whether the job's clip is already where it would go, with the sidecars it would bring.
fn is_placed(job: &Job, with_sidecars: bool) -> bool {
    let Job { clip, ref dir, prefix } = *job;
    if prefix.is_empty() && clip.path.parent() == Some(dir.as_path()) {
        return true;
    }
    let sidecars = sidecars_of(clip, with_sidecars);
    let (stem, there) = stem_in(clip, dir, prefix, sidecars);
    let original = clip.path.file_stem().unwrap_or_default();
    there && sidecars.iter().all(|sidecar| dir.join(target_name(&sidecar.path, prefix, &stem, original)).exists())
}

/// Makes `dir` and any missing parents, noting each one it made.
fn make_dir(dir: &Path, made: &mut Vec<PathBuf>) -> io::Result<()> {
    let missing: Vec<PathBuf> = dir.ancestors().take_while(|d| !d.exists()).map(Path::to_path_buf).collect();
    fs::create_dir_all(dir)?;
    made.extend(missing.into_iter().rev());
    Ok(())
}

/// A sidecar's path relative to the scanned folder, like its clip's `name`.
fn sidecar_name(clip: &Clip, sidecar: &Sidecar) -> String {
    let file = sidecar.path.file_name().unwrap_or_default();
    Path::new(&clip.name).with_file_name(file).to_string_lossy().into_owned()
}

/// The stem the clip has in `dir`, and whether it's already there under it.
/// The stem is the clip's own, or "stem (2)", "stem (3)"… when a different file has the name.
fn stem_in(clip: &Clip, dir: &Path, prefix: &str, sidecars: &[Sidecar]) -> (OsString, bool) {
    let original = clip.path.file_stem().unwrap_or_default();
    let files: Vec<&Path> = std::iter::once(clip.path.as_path())
        .chain(sidecars.iter().map(|s| s.path.as_path()))
        .collect();
    let numbered = (2..).map(|n| {
        let mut stem = original.to_owned();
        stem.push(format!(" ({n})"));
        stem
    });
    for stem in std::iter::once(original.to_owned()).chain(numbered) {
        if is_copy(&clip.path, &dir.join(target_name(&clip.path, prefix, &stem, original))) {
            return (stem, true);
        }
        if files.iter().all(|file| !dir.join(target_name(file, prefix, &stem, original)).exists()) {
            return (stem, false);
        }
    }
    unreachable!("the numbered stems never run out")
}

/// Whether `copy` is a finished copy of `file`: the same size and last-write time, which copies and moves keep.
/// The size alone isn't enough: Windows sizes a copy up front, so one cut short already has the full size.
fn is_copy(file: &Path, copy: &Path) -> bool {
    let (Ok(file), Ok(copy)) = (fs::metadata(file), fs::metadata(copy)) else { return false };
    let (Ok(a), Ok(b)) = (file.modified(), copy.modified()) else { return false };
    let gap = a.duration_since(b).unwrap_or_else(|e| e.duration());
    copy.is_file() && copy.len() == file.len() && gap <= TIME_SLACK
}

/// `prefix` + the file's own name, or + `stem` and its extension when the clip had to be numbered.
fn target_name(file: &Path, prefix: &str, stem: &OsStr, original: &OsStr) -> OsString {
    let mut name = OsString::from(prefix);
    if stem == original {
        name.push(file.file_name().unwrap_or_default());
        return name;
    }
    name.push(stem);
    if let Some(ext) = file.extension() {
        name.push(".");
        name.push(ext);
    }
    name
}

/// Moves the files of the last move back, newest first, then removes the folders it made if they're empty.
/// Files that couldn't be moved back stay in `journal`, so undo can be tried again.
pub fn undo(journal: &mut Journal, root: &Path, cancel: &AtomicBool, mut report: impl FnMut(&Progress)) -> Undone {
    let sizes: Vec<u64> = journal.moved.iter().map(|(_, to)| fs::metadata(to).map_or(0, |m| m.len())).collect();
    let verify = journal.verify;
    let passes = if verify { 2 } else { 1 };
    let mut progress = Progress {
        index: 0,
        count: journal.moved.len(),
        file: String::new(),
        bytes_done: 0,
        bytes_total: sizes.iter().sum::<u64>() * passes,
        verifying: false,
    };
    let mut undone = Undone::default();
    let mut kept = Vec::new();
    while let Some((from, to)) = journal.moved.pop() {
        if cancel.load(Ordering::Relaxed) {
            journal.moved.push((from, to));
            undone.cancelled = true;
            break;
        }
        let size = sizes[journal.moved.len()];
        progress.index += 1;
        progress.file = from.strip_prefix(root).unwrap_or(&from).to_string_lossy().into_owned();
        progress.verifying = false;
        report(&progress);

        let start = progress.bytes_done;
        let back = from.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| {
            transfer(&to, &from, Mode::Move, verify, cancel, &mut |done| {
                progress.bytes_done = start + done;
                progress.verifying = verify && (size..2 * size).contains(&done);
                report(&progress);
            })
        });
        match back {
            Ok(()) => undone.restored += 1,
            Err(_) if cancel.load(Ordering::Relaxed) => {
                journal.moved.push((from, to));
                undone.cancelled = true;
                break;
            }
            Err(e) => {
                undone.failed.push(Failure { file: progress.file.clone(), reason: crate::describe(e) });
                // A file that's gone can't be moved back, so it isn't tried again.
                if to.exists() {
                    kept.push((from, to));
                }
            }
        }
        progress.bytes_done = start + size * passes;
    }
    journal.moved.extend(kept.into_iter().rev());
    undone.remaining = journal.moved.len();
    if journal.moved.is_empty() {
        // Deepest first; remove_dir leaves any that aren't empty.
        for dir in journal.created.drain(..).rev() {
            let _ = fs::remove_dir(dir);
        }
    }
    progress.verifying = false;
    report(&progress);
    undone
}

fn mismatch() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "The copy doesn't match the original.")
}

/// Copies or moves one file, never over an existing one.
#[cfg(windows)]
fn transfer(
    src: &Path,
    dst: &Path,
    mode: Mode,
    verify: bool,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> io::Result<()> {
    use windows_sys::Win32::Foundation::ERROR_NOT_SAME_DEVICE;
    match mode {
        Mode::Copy => copy(src, dst, verify, false, cancel, on_bytes),
        // Within one drive a move is an instant rename. Across drives it's a copy, written straight to the disk
        // rather than left in the cache, and flushed with its name before the original goes.
        Mode::Move => match rename(src, dst) {
            Err(e) if e.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE as i32) => {
                copy(src, dst, verify, true, cancel, on_bytes)?;
                flush(dst)?;
                fs::remove_file(src).map_err(|e| {
                    io::Error::new(e.kind(), format!("Copied, but the original couldn't be removed: {}", crate::describe(e)))
                })
            }
            renamed => renamed,
        },
    }
}

/// Copies under a temporary name and renames it once complete, so a copy cut short by a crash or a power cut
/// never sits under the real name. Cancelling deletes it.
#[cfg(windows)]
fn copy(
    src: &Path,
    dst: &Path,
    verify: bool,
    unbuffered: bool,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> io::Result<()> {
    let mut partial = dst.as_os_str().to_owned();
    partial.push(".d-sep-partial");
    let partial = PathBuf::from(partial);
    // Left by an earlier copy that was cut short; its original is still in place. remove_file deletes it even if
    // it kept its original's read-only flag.
    let _ = fs::remove_file(&partial);
    let placed = copy_file(src, &partial, unbuffered, cancel, on_bytes)
        .and_then(|()| {
            if !verify {
                return Ok(());
            }
            let size = fs::metadata(&partial)?.len();
            compare(src, &partial, cancel, &mut |done| on_bytes(size + done))
        })
        .and_then(|()| rename(&partial, dst));
    // CopyFileExW deletes its copy when cancelled, but not always when it fails.
    if placed.is_err() {
        let _ = fs::remove_file(&partial);
    }
    placed
}

/// Writes a file's data, and its name, through to the disk itself. FlushFileBuffers needs write access, which a
/// read-only file refuses, so the flag is lifted while it runs.
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false, reason = "on Windows this only clears FILE_ATTRIBUTE_READONLY")]
fn flush(path: &Path) -> io::Result<()> {
    let permissions = fs::metadata(path)?.permissions();
    if permissions.readonly() {
        let mut writable = permissions.clone();
        writable.set_readonly(false);
        fs::set_permissions(path, writable)?;
    }
    let flushed = fs::OpenOptions::new().write(true).open(path).and_then(|file| file.sync_all());
    if permissions.readonly() {
        fs::set_permissions(path, permissions)?;
    }
    flushed
}

/// `CopyFileExW`, reporting the bytes copied, and stopping (and deleting the copy) when `cancel` is set.
#[cfg(windows)]
fn copy_file(
    src: &Path,
    dst: &Path,
    unbuffered: bool,
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> io::Result<()> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        COPY_FILE_FAIL_IF_EXISTS, COPY_FILE_NO_BUFFERING, CopyFileExW, PROGRESS_CANCEL, PROGRESS_CONTINUE,
    };

    struct Context<'a> {
        cancel: &'a AtomicBool,
        on_bytes: &'a mut dyn FnMut(u64),
    }

    unsafe extern "system" fn progress(
        _total: i64,
        done: i64,
        _stream_total: i64,
        _stream_done: i64,
        _stream: u32,
        _reason: u32,
        _src: HANDLE,
        _dst: HANDLE,
        data: *const c_void,
    ) -> u32 {
        // SAFETY: `data` is the `Context` below, which outlives the synchronous call.
        let ctx = unsafe { &mut *(data as *mut Context<'_>) };
        (ctx.on_bytes)(done as u64);
        if ctx.cancel.load(Ordering::Relaxed) { PROGRESS_CANCEL } else { PROGRESS_CONTINUE }
    }

    let (src, dst) = (verbatim(src)?, verbatim(dst)?);
    let mut ctx = Context { cancel, on_bytes };
    let data = (&raw mut ctx).cast::<c_void>();
    let flags = COPY_FILE_FAIL_IF_EXISTS | if unbuffered { COPY_FILE_NO_BUFFERING } else { 0 };
    // SAFETY: both paths are NUL-terminated, and they and `ctx` live across the call.
    let ok = unsafe { CopyFileExW(src.as_ptr(), dst.as_ptr(), Some(progress), data, std::ptr::null_mut(), flags) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// A rename within one drive. Without MOVEFILE_REPLACE_EXISTING it never overwrites, and without
/// MOVEFILE_COPY_ALLOWED it fails with ERROR_NOT_SAME_DEVICE across drives.
#[cfg(windows)]
fn rename(src: &Path, dst: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let (src, dst) = (verbatim(src)?, verbatim(dst)?);
    // SAFETY: both paths are NUL-terminated and live across the call.
    let ok = unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), 0) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Reads `copy` back from the disk itself, past the cache, and compares it with `original`.
#[cfg(windows)]
fn compare(original: &Path, copy: &Path, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64)) -> io::Result<()> {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_NO_BUFFERING;
    // Unbuffered reads need a buffer and a length aligned to the sector size; 64 KiB covers every disk.
    const ALIGN: usize = 64 * 1024;
    const CHUNK: usize = 4 * 1024 * 1024;

    let mut theirs = fs::File::open(original)?;
    let mut ours = fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_NO_BUFFERING).open(copy)?;
    if theirs.metadata()?.len() != ours.metadata()?.len() {
        return Err(mismatch());
    }
    let mut raw = vec![0u8; CHUNK + ALIGN];
    let skew = raw.as_ptr().align_offset(ALIGN);
    let read_back = &mut raw[skew..skew + CHUNK];
    let mut expected = vec![0u8; CHUNK];
    let mut done = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let n = ours.read(read_back)?;
        if n == 0 {
            return Ok(());
        }
        theirs.read_exact(&mut expected[..n])?;
        if read_back[..n] != expected[..n] {
            return Err(mismatch());
        }
        done += n as u64;
        on_bytes(done);
    }
}

/// `path` as a NUL-terminated wide string with the `\\?\` prefix, which lifts the 260-character limit.
/// Rust's own fs functions add it themselves; the raw calls here need it too.
#[cfg(windows)]
fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    // Also resolves "." and ".." and turns "/" into "\", which Windows doesn't do for \\?\ paths.
    let path: Vec<u16> = std::path::absolute(path)?.as_os_str().encode_wide().collect();
    let wide = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let mut out = if path.starts_with(&wide(r"\\?\")) || path.starts_with(&wide(r"\\.\")) {
        path
    } else if let Some(share) = path.strip_prefix(&wide(r"\\")[..]) {
        [&wide(r"\\?\UNC\")[..], share].concat()
    } else {
        [&wide(r"\\?\")[..], &path].concat()
    };
    out.push(0);
    Ok(out)
}

#[cfg(not(windows))]
fn transfer(
    src: &Path,
    dst: &Path,
    mode: Mode,
    verify: bool,
    _cancel: &AtomicBool,
    _on_bytes: &mut dyn FnMut(u64),
) -> io::Result<()> {
    if dst.exists() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    if mode == Mode::Move {
        return fs::rename(src, dst);
    }
    fs::copy(src, dst)?;
    // Keep the time, as CopyFileExW does, so `is_copy` knows the copy.
    fs::File::options().write(true).open(dst)?.set_modified(fs::metadata(src)?.modified()?)?;
    if verify && fs::read(src)? != fs::read(dst)? {
        let _ = fs::remove_file(dst);
        return Err(mismatch());
    }
    Ok(())
}

/// Bytes free on the drive that holds `dir`.
#[cfg(windows)]
pub fn free_space(dir: &Path) -> io::Result<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let dir = verbatim(dir)?;
    let mut free = 0;
    // SAFETY: the path is NUL-terminated; the outputs not wanted may be null.
    let ok = unsafe { GetDiskFreeSpaceExW(dir.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(free) }
}

/// Whether two folders are on the same drive, where a move is just a rename.
#[cfg(windows)]
pub fn same_drive(a: &Path, b: &Path) -> io::Result<bool> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;
    let volume = |path: &Path| -> io::Result<String> {
        let path = verbatim(path)?;
        let mut buf = vec![0u16; path.len() + 1];
        // SAFETY: the path is NUL-terminated, and the buffer, longer than it, is passed with its length.
        let ok = unsafe { GetVolumePathNameW(path.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(OsString::from_wide(&buf[..len]).to_string_lossy().to_lowercase())
    };
    Ok(volume(a)? == volume(b)?)
}

#[cfg(not(windows))]
pub fn free_space(_dir: &Path) -> io::Result<u64> {
    Ok(u64::MAX)
}

#[cfg(not(windows))]
pub fn same_drive(_a: &Path, _b: &Path) -> io::Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::tests::{fake_clip, temp_dir};
    use crate::scan::scan;
    use std::path::PathBuf;

    fn footage(name: &str) -> PathBuf {
        let root = temp_dir(name);
        fs::write(root.join("A.MP4"), fake_clip(Some("D-Log"))).unwrap();
        fs::write(root.join("A.LRF"), b"proxy A").unwrap();
        fs::write(root.join("B.MP4"), fake_clip(Some("D-Log2"))).unwrap();
        fs::write(root.join("B.SRT"), b"telemetry B").unwrap();
        fs::write(root.join("C.MP4"), fake_clip(Some("D-Log M"))).unwrap();
        fs::write(root.join("D.MP4"), b"not DJI").unwrap();
        fs::write(root.join("H.MP4"), fake_clip(Some("Rec.2100 HLG"))).unwrap();
        root
    }

    fn options(mode: Mode, action: Action, sidecars: bool) -> Options {
        let prefixes = Prefixes { dlog: "DL_".into(), dlog2: "DL2_".into(), dlogm: "DLM_".into() };
        Options { mode, action, sidecars, prefixes, destination: None, layout: Layout::Gather, verify: false }
    }

    fn sort_as(root: &Path, mode: Mode, action: Action) -> Summary {
        let clips = scan(root, true).unwrap();
        run(root, &clips, &options(mode, action, true), &AtomicBool::new(false), |_| {}).summary
    }

    fn sort(root: &Path, mode: Mode, sidecars: bool) -> Summary {
        let clips = scan(root, false).unwrap();
        run(root, &clips, &options(mode, Action::Folders, sidecars), &AtomicBool::new(false), |_| {}).summary
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn copies_log_clips_with_sidecars() {
        let root = footage("copy");
        let summary = sort(&root, Mode::Copy, true);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (1, 1, 1, 2));
        assert!(summary.failed.is_empty() && !summary.cancelled);
        assert_eq!(listing(&root.join(DLOG_DIR)), ["A.LRF", "A.MP4"]);
        assert_eq!(listing(&root.join(DLOG2_DIR)), ["B.MP4", "B.SRT"]);
        assert_eq!(listing(&root.join(DLOGM_DIR)), ["C.MP4"]);
        assert_eq!(fs::read(root.join(DLOG_DIR).join("A.MP4")).unwrap(), fake_clip(Some("D-Log")));
        // Originals and non-log clips stay put.
        assert_eq!(
            listing(&root),
            ["A.LRF", "A.MP4", "B.MP4", "B.SRT", "C.MP4", "D-Log", "D-Log 2", "D-Log M", "D.MP4", "H.MP4"]
        );

        // A second run finds everything already in place.
        let again = sort(&root, Mode::Copy, true);
        assert_eq!((again.dlog, again.dlog2, again.dlogm, again.existing), (0, 0, 0, 3));
        assert_eq!(listing(&root.join(DLOG_DIR)), ["A.LRF", "A.MP4"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn moves_log_clips() {
        let root = footage("move");
        let summary = sort(&root, Mode::Move, true);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (1, 1, 1, 2));
        assert_eq!(listing(&root), ["D-Log", "D-Log 2", "D-Log M", "D.MP4", "H.MP4"]);
        assert_eq!(listing(&root.join(DLOG2_DIR)), ["B.MP4", "B.SRT"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn leaves_sidecars_when_asked() {
        let root = footage("nosidecars");
        let summary = sort(&root, Mode::Move, false);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (1, 1, 1, 0));
        assert_eq!(listing(&root), ["A.LRF", "B.SRT", "D-Log", "D-Log 2", "D-Log M", "D.MP4", "H.MP4"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn never_overwrites_a_different_file() {
        let root = footage("collide");
        fs::create_dir_all(root.join(DLOG_DIR)).unwrap();
        fs::write(root.join(DLOG_DIR).join("A.MP4"), b"someone else's clip").unwrap();
        fs::write(root.join(DLOG_DIR).join("A (2).LRF"), b"blocks the (2) slot").unwrap();
        let summary = sort(&root, Mode::Move, true);
        assert_eq!(summary.dlog, 1);
        assert_eq!(
            listing(&root.join(DLOG_DIR)),
            ["A (2).LRF", "A (3).LRF", "A (3).MP4", "A.MP4"]
        );
        assert_eq!(fs::read(root.join(DLOG_DIR).join("A.MP4")).unwrap(), b"someone else's clip");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renames_in_place() {
        let root = footage("rename");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("E.MP4"), fake_clip(Some("D-Log"))).unwrap();
        let summary = sort_as(&root, Mode::Move, Action::Rename);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (2, 1, 1, 2));
        assert!(summary.failed.is_empty());
        assert_eq!(
            listing(&root),
            ["D.MP4", "DL2_B.MP4", "DL2_B.SRT", "DLM_C.MP4", "DL_A.LRF", "DL_A.MP4", "H.MP4", "sub"]
        );
        assert_eq!(listing(&root.join("sub")), ["DL_E.MP4"]);

        // Already prefixed clips aren't prefixed again.
        let again = sort_as(&root, Mode::Move, Action::Rename);
        assert_eq!((again.dlog, again.dlog2, again.dlogm, again.existing), (0, 0, 0, 4));
        assert_eq!(listing(&root.join("sub")), ["DL_E.MP4"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copies_with_new_names() {
        let root = footage("renamecopy");
        let summary = sort_as(&root, Mode::Copy, Action::Rename);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (1, 1, 1, 2));
        assert_eq!(
            listing(&root),
            [
                "A.LRF", "A.MP4", "B.MP4", "B.SRT", "C.MP4", "D.MP4", "DL2_B.MP4", "DL2_B.SRT", "DLM_C.MP4", "DL_A.LRF",
                "DL_A.MP4", "H.MP4"
            ]
        );
        assert_eq!(fs::read(root.join("DL_A.MP4")).unwrap(), fake_clip(Some("D-Log")));

        // The originals find their copies, and the copies already have their prefix.
        let again = sort_as(&root, Mode::Copy, Action::Rename);
        assert_eq!((again.dlog, again.dlog2, again.dlogm, again.existing), (0, 0, 0, 6));
        assert_eq!(listing(&root).len(), 12);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sorts_and_renames() {
        let root = footage("both");
        fs::write(root.join("dl_F.MP4"), fake_clip(Some("D-Log"))).unwrap();
        let summary = sort_as(&root, Mode::Move, Action::Both);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (2, 1, 1, 2));
        assert_eq!(listing(&root), ["D-Log", "D-Log 2", "D-Log M", "D.MP4", "H.MP4"]);
        assert_eq!(listing(&root.join(DLOG_DIR)), ["DL_A.LRF", "DL_A.MP4", "dl_F.MP4"]);
        assert_eq!(listing(&root.join(DLOG2_DIR)), ["DL2_B.MP4", "DL2_B.SRT"]);
        assert_eq!(listing(&root.join(DLOGM_DIR)), ["DLM_C.MP4"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renaming_never_overwrites() {
        let root = footage("renamecollide");
        fs::write(root.join("DL_A.MP4"), b"someone else's clip").unwrap();
        let summary = sort_as(&root, Mode::Move, Action::Rename);
        assert_eq!(summary.dlog, 1);
        assert!(root.join("DL_A (2).MP4").exists() && root.join("DL_A (2).LRF").exists());
        assert_eq!(fs::read(root.join("DL_A.MP4")).unwrap(), b"someone else's clip");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsafe_prefixes() {
        let with = |dlog: &str| Options {
            prefixes: Prefixes { dlog: dlog.into(), dlog2: "DL2_".into(), dlogm: "DLM_".into() },
            ..options(Mode::Move, Action::Rename, true)
        };
        assert!(with("DL_").check().is_ok());
        assert!(with("Log 1 - ").check().is_ok());
        assert!(with(&"x".repeat(16)).check().is_ok());
        for fine in ["CON", "NULL.", "CONX.", "COM.", "COM10.", "AUX_", "😀".repeat(16).as_str()] {
            assert!(with(fine).check().is_ok(), "{fine:?} was refused");
        }
        for bad in ["", "  ", "..\\", "a/b", "C:", "a*", ".x", "$x", " x", "a\tb", "a\u{9f}b", "x".repeat(17).as_str()] {
            assert!(with(bad).check().is_err(), "{bad:?} was accepted");
        }
        for device in ["NUL.", "con.x", "Aux.", "PRN .", "COM1.", "lpt9.a"] {
            assert!(with(device).check().unwrap_err().ends_with("device name like CON or NUL."), "{device:?}");
        }
        assert!(with("a\u{7f}").check().unwrap_err().ends_with("can't contain control characters."));
        let mut slash_in_m = with("DL_");
        slash_in_m.prefixes.dlogm = "M/".into();
        assert!(slash_in_m.check().unwrap_err().starts_with("The D-Log M prefix"));
        // Prefixes don't matter when only sorting into folders.
        assert!(Options { action: Action::Folders, ..with("a/b") }.check().is_ok());
    }

    #[test]
    fn stops_when_cancelled() {
        let root = footage("cancel");
        let clips = scan(&root, false).unwrap();
        let summary = run(&root, &clips, &options(Mode::Copy, Action::Folders, true), &AtomicBool::new(true), |_| {}).summary;
        assert!(summary.cancelled);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm), (0, 0, 0));
        assert!(!root.join(DLOG_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn cancelling_mid_copy_removes_the_partial_file() {
        let root = temp_dir("midcopy");
        let mut clip = fake_clip(Some("D-Log"));
        clip.resize(256 * 1024 * 1024, 0);
        fs::write(root.join("A.MP4"), &clip).unwrap();
        let clips = scan(&root, false).unwrap();
        let cancel = AtomicBool::new(false);
        let mut partial = false;
        let summary = run(&root, &clips, &options(Mode::Copy, Action::Folders, true), &cancel, |p| {
            if p.bytes_done > 0 && p.bytes_done < p.bytes_total {
                partial = true;
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .summary;
        assert!(partial, "copy never reported partial progress");
        assert!(summary.cancelled && summary.dlog == 0);
        // The folder made for it went with it.
        assert!(!root.join(DLOG_DIR).exists());
        assert_eq!(fs::metadata(root.join("A.MP4")).unwrap().len(), clip.len() as u64);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_copy_cut_short_is_not_taken_for_the_clip() {
        let root = footage("cutshort");
        // Cut short, a copy has the clip's full size (Windows sizes it up front) but a later time.
        let clip = root.join("A.MP4");
        let len = fs::metadata(&clip).unwrap().len() as usize;
        let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        fs::File::options().write(true).open(&clip).unwrap().set_modified(hour_ago).unwrap();
        fs::create_dir_all(root.join(DLOG_DIR)).unwrap();
        fs::write(root.join(DLOG_DIR).join("A.MP4"), vec![0; len]).unwrap();

        let summary = sort(&root, Mode::Copy, true);
        assert_eq!((summary.dlog, summary.existing), (1, 0));
        assert_eq!(listing(&root.join(DLOG_DIR)), ["A (2).LRF", "A (2).MP4", "A.MP4"]);
        assert_eq!(fs::read(root.join(DLOG_DIR).join("A (2).MP4")).unwrap(), fs::read(&clip).unwrap());

        // A second run finds the finished copy under its numbered name.
        let again = sort(&root, Mode::Copy, true);
        assert_eq!((again.dlog, again.existing, again.sidecars), (0, 3, 0));
        assert_eq!(listing(&root.join(DLOG_DIR)).len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn copies_under_a_temporary_name() {
        let root = footage("partial");
        fs::create_dir_all(root.join(DLOG2_DIR)).unwrap();
        fs::write(root.join(DLOG2_DIR).join("B.MP4.d-sep-partial"), b"left by a copy cut short").unwrap();
        let summary = sort(&root, Mode::Copy, true);
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (1, 1, 1, 2));
        assert_eq!(listing(&root.join(DLOG2_DIR)), ["B.MP4", "B.SRT"]);
        assert_eq!(fs::read(root.join(DLOG2_DIR).join("B.MP4")).unwrap(), fake_clip(Some("D-Log2")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn brings_sidecars_to_clips_already_in_place() {
        let root = footage("latesidecars");
        sort(&root, Mode::Copy, false);
        assert_eq!(listing(&root.join(DLOG_DIR)), ["A.MP4"]);
        let again = sort(&root, Mode::Copy, true);
        assert_eq!((again.dlog, again.existing, again.sidecars), (0, 3, 2));
        assert_eq!(listing(&root.join(DLOG_DIR)), ["A.LRF", "A.MP4"]);
        assert_eq!(listing(&root.join(DLOG2_DIR)), ["B.MP4", "B.SRT"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_move_leaves_a_clip_already_in_place_with_its_sidecars() {
        let root = footage("latesidecarsmove");
        sort(&root, Mode::Copy, false);
        // The clips stay put, because they're already in their folders; their sidecars are copied to the copies.
        let again = sort(&root, Mode::Move, true);
        assert_eq!((again.dlog, again.existing, again.sidecars), (0, 3, 2));
        assert!(root.join("A.MP4").exists() && root.join("A.LRF").exists());
        assert_eq!(listing(&root.join(DLOG_DIR)), ["A.LRF", "A.MP4"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn knows_which_clips_are_already_in_place() {
        let root = footage("inplace");
        let clips = scan(&root, false).unwrap();
        let names = |options: &Options| -> Vec<String> {
            let paths = in_place(&root, &clips, options);
            paths.iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().into_owned()).collect()
        };
        assert!(names(&options(Mode::Copy, Action::Folders, true)).is_empty());

        // Copied without sidecars, the clips are in place only as long as their sidecars aren't wanted.
        sort(&root, Mode::Copy, false);
        assert_eq!(names(&options(Mode::Copy, Action::Folders, false)), ["A.MP4", "B.MP4", "C.MP4"]);
        assert_eq!(names(&options(Mode::Copy, Action::Folders, true)), ["C.MP4"]);
        assert!(names(&options(Mode::Copy, Action::Both, true)).is_empty());

        // A clip renamed in place is in place once it has its prefix.
        fs::rename(root.join("C.MP4"), root.join("DLM_C.MP4")).unwrap();
        let clips = scan(&root, false).unwrap();
        let renamed: Vec<_> = in_place(&root, &clips, &options(Mode::Move, Action::Rename, true));
        assert_eq!(renamed, [root.join("DLM_C.MP4")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn flushes_a_read_only_file_and_keeps_it_read_only() {
        let dir = temp_dir("flush");
        let file = dir.join("A.MP4");
        fs::write(&file, b"copied").unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();
        flush(&file).unwrap();
        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sorting_an_output_folder_does_not_nest_it() {
        let parent = temp_dir("outputroot");
        let root = parent.join(DLOG_DIR);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("A.MP4"), fake_clip(Some("D-Log"))).unwrap();
        fs::write(root.join("B.MP4"), fake_clip(Some("D-Log2"))).unwrap();
        let summary = sort_as(&root, Mode::Move, Action::Folders);
        assert_eq!((summary.dlog, summary.dlog2, summary.existing), (0, 1, 1));
        assert_eq!(listing(&root), ["A.MP4"]);
        assert_eq!(listing(&parent.join(DLOG2_DIR)), ["B.MP4"]);

        // With a prefix, the D-Log clip is renamed where it is.
        let summary = sort_as(&root, Mode::Move, Action::Both);
        assert_eq!(summary.dlog, 1);
        assert_eq!(listing(&root), ["DL_A.MP4"]);
        fs::remove_dir_all(parent).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn handles_paths_longer_than_260_characters() {
        let base = temp_dir("longpath");
        let root = base.join("x".repeat(120)).join("y".repeat(120));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("A.MP4"), fake_clip(Some("D-Log"))).unwrap();
        fs::write(root.join("A.LRF"), b"proxy").unwrap();
        assert!(root.join(DLOG_DIR).join("A.MP4").as_os_str().len() > 260);

        let summary = sort(&root, Mode::Copy, true);
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);
        assert_eq!((summary.dlog, summary.sidecars), (1, 1));
        let summary = sort_as(&root, Mode::Move, Action::Rename);
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);
        assert_eq!(listing(&root), ["D-Log", "DL_A.LRF", "DL_A.MP4"]);
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn names_failed_sidecars_by_their_path() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temp_dir("lockedsidecars");
        for (card, padding) in [("card1", 0), ("card2", 10)] {
            fs::create_dir_all(root.join(card)).unwrap();
            let mut clip = fake_clip(Some("D-Log"));
            clip.extend(vec![0; padding]);
            fs::write(root.join(card).join("A.MP4"), clip).unwrap();
            fs::write(root.join(card).join("A.LRF"), b"proxy").unwrap();
        }
        // Held open with no sharing, so they can't be copied.
        let lock = |card: &str| fs::OpenOptions::new().read(true).share_mode(0).open(root.join(card).join("A.LRF"));
        let locks = [lock("card1").unwrap(), lock("card2").unwrap()];
        let summary = sort_as(&root, Mode::Copy, Action::Folders);
        drop(locks);
        let failed: Vec<_> = summary.failed.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(failed, [r"card1\A.LRF", r"card2\A.LRF"]);
        assert_eq!(summary.dlog, 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_progress_up_to_the_total() {
        let root = footage("progress");
        let clips = scan(&root, false).unwrap();
        let mut last = None;
        let options = options(Mode::Copy, Action::Folders, true);
        run(&root, &clips, &options, &AtomicBool::new(false), |p| last = Some(p.clone()));
        let last = last.unwrap();
        assert_eq!((last.index, last.count), (3, 3));
        assert_eq!(last.bytes_done, last.bytes_total);

        // Verifying reads every copy back, so it's twice the work, and says when it's at it.
        for dir in [DLOG_DIR, DLOG2_DIR, DLOGM_DIR] {
            fs::remove_dir_all(root.join(dir)).unwrap();
        }
        let total = last.bytes_total;
        let (mut last, mut seen, mut verifying) = (None, 0, false);
        let verified = Options { verify: true, ..options };
        run(&root, &clips, &verified, &AtomicBool::new(false), |p| {
            seen = seen.max(p.bytes_done);
            verifying |= p.verifying;
            last = Some(p.clone());
        });
        let last = last.unwrap();
        assert_eq!((last.bytes_total, last.bytes_done, seen), (2 * total, 2 * total, 2 * total));
        // Only the Windows copy reports its bytes as they go.
        assert!(!last.verifying && (verifying || !cfg!(windows)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn progress_leaves_out_clips_already_in_place() {
        let root = footage("progressplaced");
        let clips = scan(&root, false).unwrap();
        let options = options(Mode::Copy, Action::Folders, true);
        // A is copied already, as when a sort was cancelled after it.
        run(&root, &clips[..1], &options, &AtomicBool::new(false), |_| {});
        let mut first = None;
        let summary = run(&root, &clips, &options, &AtomicBool::new(false), |p| {
            first.get_or_insert_with(|| p.clone());
        })
        .summary;
        // B, its sidecar and C: the work left, as the UI's button counts it.
        let first = first.unwrap();
        let left = clips[1].size + clips[1].sidecars[0].size + clips[2].size;
        assert_eq!((first.index, first.count, first.file.as_str(), first.bytes_total), (1, 2, "B.MP4", left));
        assert_eq!((summary.dlog2, summary.dlogm, summary.existing), (1, 1, 1));
        // A still counts as in its folder, which the result screen opens.
        assert_eq!(summary.folders.dlog, Some(root.join(DLOG_DIR)));
        fs::remove_dir_all(root).unwrap();
    }

    /// `footage`, plus a D-Log clip with a sidecar in a subfolder.
    fn footage_with_sub(name: &str) -> PathBuf {
        let root = footage(name);
        fs::create_dir_all(root.join("sub")).unwrap();
        let mut clip = fake_clip(Some("D-Log"));
        clip.extend([0; 10]);
        fs::write(root.join("sub").join("E.MP4"), clip).unwrap();
        fs::write(root.join("sub").join("E.SRT"), b"telemetry E").unwrap();
        root
    }

    fn sort_with(root: &Path, options: &Options) -> Outcome {
        run(root, &scan(root, true).unwrap(), options, &AtomicBool::new(false), |_| {})
    }

    #[test]
    fn sorts_into_a_destination_keeping_subfolders() {
        let root = footage_with_sub("dest-src");
        let dest = temp_dir("dest-dst");
        let into = |action| Options { destination: Some(dest.clone()), verify: true, ..options(Mode::Copy, action, true) };

        let summary = sort_with(&root, &into(Action::Folders)).summary;
        assert_eq!((summary.dlog, summary.dlog2, summary.dlogm, summary.sidecars), (2, 1, 1, 3));
        assert_eq!(listing(&dest.join(DLOG_DIR)), ["A.LRF", "A.MP4", "E.MP4", "E.SRT"]);
        assert_eq!(summary.folder, dest);
        assert_eq!(summary.folders.dlog, Some(dest.join(DLOG_DIR)));
        assert!(!root.join(DLOG_DIR).exists(), "nothing goes to the scanned folder");

        // Names keep each clip's subfolder, under the destination.
        let summary = sort_with(&root, &into(Action::Rename)).summary;
        assert_eq!(summary.dlog, 2);
        assert_eq!(listing(&dest.join("sub")), ["DL_E.MP4", "DL_E.SRT"]);
        assert!(dest.join("DL_A.MP4").exists() && root.join("A.MP4").exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(dest).unwrap();
    }

    #[test]
    fn leaves_clips_in_a_destination_inside_the_folder_where_they_are() {
        for (name, action, layout) in [("inside-names", Action::Rename, Layout::Gather), ("inside-each", Action::Folders, Layout::Each)] {
            let root = footage_with_sub(name);
            let dest = root.join("Out");
            fs::create_dir_all(&dest).unwrap();
            let into = Options { destination: Some(dest.clone()), layout, ..options(Mode::Copy, action, true) };
            sort_with(&root, &into);
            let sorted = listing(&dest);
            // Sorting again finds the copies in the destination too, already where they belong: no Out\Out.
            let again = sort_with(&root, &into).summary;
            assert_eq!((again.dlog, again.dlog2, again.dlogm, again.existing), (0, 0, 0, 8), "{name}");
            assert_eq!(listing(&dest), sorted, "{name}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    /// Windows ignores case in paths, so a destination spelled unlike the scanned folder is still inside it.
    #[cfg(windows)]
    #[test]
    fn finds_a_destination_inside_the_folder_whatever_its_case() {
        for (name, action, layout) in [("case-names", Action::Rename, Layout::Gather), ("case-each", Action::Folders, Layout::Each)] {
            let root = footage_with_sub(name);
            let dest = root.join("Out");
            fs::create_dir_all(&dest).unwrap();
            // The scan spells the folder in capitals, the destination as it is.
            let shouted = PathBuf::from(root.to_string_lossy().to_uppercase());
            let into = Options { destination: Some(dest.clone()), layout, ..options(Mode::Move, action, true) };
            sort_with(&shouted, &into);
            let sorted = listing(&dest);
            // The clips moved into the destination are found there again, already where they belong: no Out\Out.
            let again = sort_with(&shouted, &into).summary;
            assert_eq!((again.dlog, again.dlog2, again.dlogm, again.existing), (0, 0, 0, 4), "{name}");
            assert_eq!(listing(&dest), sorted, "{name}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn sorts_inside_each_subfolder() {
        let root = footage_with_sub("each");
        let each = Options { layout: Layout::Each, ..options(Mode::Move, Action::Folders, true) };
        let summary = sort_with(&root, &each).summary;
        assert_eq!((summary.dlog, summary.sidecars), (2, 3));
        assert_eq!(listing(&root.join(DLOG_DIR)), ["A.LRF", "A.MP4"]);
        assert_eq!(listing(&root.join("sub").join(DLOG_DIR)), ["E.MP4", "E.SRT"]);
        // The D-Log clips are in two folders now, so there isn't one to open.
        assert_eq!(summary.folders.dlog, None);
        assert_eq!(summary.folders.dlog2, Some(root.join(DLOG2_DIR)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn undoes_a_move() {
        let root = footage_with_sub("undo");
        let before = (listing(&root), listing(&root.join("sub")));
        let mut outcome = sort_with(&root, &options(Mode::Move, Action::Both, true));
        assert!(outcome.summary.undoable);
        assert_eq!(listing(&root.join(DLOG_DIR)), ["DL_A.LRF", "DL_A.MP4", "DL_E.MP4", "DL_E.SRT"]);

        let undone = undo(&mut outcome.journal, &root, &AtomicBool::new(false), |_| {});
        assert_eq!((undone.restored, undone.remaining, undone.cancelled), (7, 0, false));
        assert!(undone.failed.is_empty(), "{:?}", undone.failed);
        // Everything is back, and the folders the sort made are gone.
        assert_eq!((listing(&root), listing(&root.join("sub"))), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn undo_keeps_what_it_cannot_put_back() {
        let root = footage("undoblocked");
        let mut outcome = sort_with(&root, &options(Mode::Move, Action::Folders, true));
        // A new file where a clip used to be.
        fs::write(root.join("B.MP4"), b"a new file").unwrap();

        let undone = undo(&mut outcome.journal, &root, &AtomicBool::new(false), |_| {});
        assert_eq!((undone.restored, undone.remaining), (4, 1));
        assert_eq!(undone.failed.len(), 1);
        assert_eq!(undone.failed[0].file, "B.MP4");
        assert_eq!(fs::read(root.join("B.MP4")).unwrap(), b"a new file");
        assert_eq!(listing(&root.join(DLOG2_DIR)), ["B.MP4"]);

        // Once the way is clear, a second undo finishes the job.
        fs::remove_file(root.join("B.MP4")).unwrap();
        let undone = undo(&mut outcome.journal, &root, &AtomicBool::new(false), |_| {});
        assert_eq!((undone.restored, undone.remaining), (1, 0));
        assert!(!root.join(DLOG2_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copies_are_not_undoable() {
        let root = footage("copynoundo");
        let outcome = sort_with(&root, &options(Mode::Copy, Action::Folders, true));
        assert!(!outcome.summary.undoable && outcome.journal.moved.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_a_missing_destination() {
        let dir = temp_dir("gone");
        let gone = Options { destination: Some(dir.join("nope")), ..options(Mode::Copy, Action::Folders, true) };
        assert_eq!(gone.check().unwrap_err(), "The destination folder no longer exists.");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn compares_copies_with_their_originals() {
        let dir = temp_dir("compare");
        // Bigger than one read, and not a whole number of sectors.
        let data: Vec<u8> = (0..5_000_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(dir.join("a"), &data).unwrap();
        fs::write(dir.join("same"), &data).unwrap();
        let mut changed = data.clone();
        changed[4_999_999] ^= 1;
        fs::write(dir.join("changed"), &changed).unwrap();

        let mut done = 0;
        compare(&dir.join("a"), &dir.join("same"), &AtomicBool::new(false), &mut |n| done = n).unwrap();
        assert_eq!(done, data.len() as u64);
        let err = compare(&dir.join("a"), &dir.join("changed"), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_free_space_and_drives() {
        let dir = temp_dir("space");
        fs::create_dir_all(dir.join("sub")).unwrap();
        assert!(free_space(&dir).unwrap() > 0);
        assert!(same_drive(&dir, &dir.join("sub")).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    /// The JSON the UI sends and gets back; src/lib/api.ts mirrors it by hand.
    #[test]
    fn speaks_the_ui_json() {
        let options: Options = serde_json::from_str(
            r#"{"mode":"move","action":"both","sidecars":true,"prefixes":{"dlog":"A","dlog2":"B","dlogm":"C"},
                "destination":null,"layout":"each","verify":true}"#,
        )
        .unwrap();
        assert_eq!((options.mode, options.action, options.layout), (Mode::Move, Action::Both, Layout::Each));
        assert!(options.verify && options.destination.is_none());

        let summary = Summary { folders: ProfileFolders { dlog: Some("X".into()), ..Default::default() }, ..Default::default() };
        let json = serde_json::to_value(&summary).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["cancelled", "dlog", "dlog2", "dlogm", "existing", "failed", "folder", "folders", "sidecars", "undoable"]
        );
        assert_eq!(json["folders"], serde_json::json!({ "dlog": "X", "dlog2": null, "dlogm": null }));
        let undone = serde_json::to_value(Undone::default()).unwrap();
        assert_eq!(undone, serde_json::json!({ "restored": 0, "remaining": 0, "failed": [], "cancelled": false }));
    }
}
