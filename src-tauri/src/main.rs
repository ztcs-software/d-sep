// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod probe;
mod scan;
mod sort;

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager, State, UserAttentionType, WebviewWindow, WindowEvent};

#[derive(Default)]
struct AppState {
    /// The last scan; sorting works from it so the UI can't ask to move arbitrary files.
    scan: Mutex<Option<Scan>>,
    /// The folder of the last scan, still known after a sort has taken the scan.
    root: Mutex<Option<PathBuf>>,
    /// The folders the UI may open: the last scanned one, and those the last sort put clips in.
    openable: Mutex<Vec<PathBuf>>,
    /// What the last move did, for `undo_sort`.
    journal: Mutex<Option<sort::Journal>>,
    /// The sort or undo under way, and its cancel flag.
    work: Mutex<Work>,
    /// The id of the scan to stop. The UI numbers its scans, so a Cancel can't stop the next one.
    scan_cancel: Arc<AtomicU64>,
    /// Set while a sort or an undo runs, so closing the window asks first.
    working: AtomicBool,
    /// "Stop and quit" was chosen: quit once the work has stopped.
    quit: AtomicBool,
    /// The "Stop and quit?" question is showing.
    asking: AtomicBool,
}

impl AppState {
    /// Keeps a finished scan to sort from. It starts afresh: only its folder may be opened, and the last move is no
    /// longer offered for undoing.
    fn scanned(&self, scan: &Scan) {
        *self.scan.lock().unwrap() = Some(scan.clone());
        *self.root.lock().unwrap() = Some(scan.root.clone());
        *self.openable.lock().unwrap() = vec![scan.root.clone()];
        *self.journal.lock().unwrap() = None;
    }

    /// After a sort: the folders it put clips in may be opened, and a move is kept for undoing.
    fn sorted(&self, summary: &sort::Summary, journal: sort::Journal) {
        let folders = &summary.folders;
        let used = [Some(&summary.folder), folders.dlog.as_ref(), folders.dlog2.as_ref(), folders.dlogm.as_ref()];
        self.openable.lock().unwrap().extend(used.into_iter().flatten().cloned());
        *self.journal.lock().unwrap() = summary.undoable.then_some(journal);
    }

    /// After an undo, which took the journal: what it couldn't put back is kept, to try again.
    fn undone(&self, undone: &sort::Undone, journal: sort::Journal) {
        if undone.remaining > 0 {
            *self.journal.lock().unwrap() = Some(journal);
        }
    }
}

/// A sort or an undo, by the id the UI gave it. Each gets its own cancel flag, so a Cancel that arrives after its
/// work has ended can't stop the next one, and one that arrives before its work has started still stops it.
#[derive(Default)]
struct Work {
    id: u64,
    cancel: Arc<AtomicBool>,
    /// The last id the UI asked to stop.
    stopped: u64,
}

impl Work {
    /// Starts work `id`, already cancelled if its Cancel came first.
    fn start(&mut self, id: u64) -> Arc<AtomicBool> {
        self.id = id;
        self.cancel = Arc::new(AtomicBool::new(self.stopped == id));
        self.cancel.clone()
    }

    fn stop(&mut self, id: u64) {
        self.stopped = id;
        if self.id == id {
            self.cancel.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Scan {
    root: PathBuf,
    clips: Vec<scan::Clip>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiskSpace {
    /// Bytes free on the drive the clips would go to.
    free: u64,
    /// Whether that's the drive the clips are on, where a move is just a rename.
    same_drive: bool,
}

/// Scans a folder, reporting progress. None when `cancel_scan` stopped it.
#[tauri::command]
async fn scan_folder(
    state: State<'_, AppState>,
    path: PathBuf,
    recursive: bool,
    id: u64,
    on_progress: Channel<scan::ScanProgress>,
) -> Result<Option<Scan>, String> {
    // A scan replaces the journal an undo puts back, and the UI never asks for one while it's busy.
    if state.working.load(Ordering::SeqCst) {
        return Err("Wait for D-Sep to finish first.".into());
    }
    let cancel = state.scan_cancel.clone();
    let scan = tauri::async_runtime::spawn_blocking(move || {
        // A dropped file means "its folder".
        let root = if path.is_file() { path.parent().map(Path::to_path_buf).unwrap_or(path) } else { path };
        // Probe threads report too, so the last send time is shared. ~10 updates a second suits a counter.
        let last = Mutex::new(None::<Instant>);
        let report = |progress: scan::ScanProgress| {
            let mut last = last.lock().unwrap();
            if last.is_none_or(|at| at.elapsed() >= Duration::from_millis(100)) {
                *last = Some(Instant::now());
                let _ = on_progress.send(progress);
            }
        };
        let cancelled = || cancel.load(Ordering::SeqCst) == id;
        let watch = scan::Watch { report: &report, cancelled: &cancelled };
        scan::scan_watched(&root, recursive, &watch).map(|clips| clips.map(|clips| Scan { root, clips }))
    })
    .await
    .map_err(describe)?
    .map_err(describe)?;
    // A cancelled scan leaves everything as it was, the last move's undo included.
    if let Some(scan) = &scan {
        state.scanned(scan);
    }
    Ok(scan)
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>, id: u64) {
    state.scan_cancel.store(id, Ordering::SeqCst);
}

/// The clips of the last scan that a sort with `options` would leave alone, because they're already in place.
#[tauri::command]
async fn in_place(state: State<'_, AppState>, options: sort::Options) -> Result<Vec<PathBuf>, String> {
    options.check()?;
    let Some(scan) = state.scan.lock().unwrap().clone() else { return Ok(Vec::new()) };
    // It looks at a few files per clip, which on a card or a network drive can take a moment.
    tauri::async_runtime::spawn_blocking(move || sort::in_place(&scan.root, &scan.clips, &options))
        .await
        .map_err(describe)
}

/// Sorts the clips of the last scan. `id` numbers it for `cancel_sort`.
#[tauri::command]
async fn sort_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    options: sort::Options,
    id: u64,
    on_progress: Channel<sort::Progress>,
) -> Result<sort::Summary, String> {
    // Checked before taking the scan, so fixing a prefix doesn't need a rescan.
    options.check()?;
    let scan = state.scan.lock().unwrap().take().ok_or("Scan the folder again first.")?;
    let cancel = state.work.lock().unwrap().start(id);
    let mut reporter = Reporter::new(&app, on_progress);
    state.working.store(true, Ordering::SeqCst);
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let outcome = sort::run(&scan.root, &scan.clips, &options, &cancel, |progress| reporter.report(progress));
        reporter.finish();
        outcome
    })
    .await;
    finish_work(&app, &state);
    let sort::Outcome { summary, journal } = outcome.map_err(describe)?;
    state.sorted(&summary, journal);
    Ok(summary)
}

/// Stops the sort or undo numbered `id`.
#[tauri::command]
fn cancel_sort(state: State<'_, AppState>, id: u64) {
    state.work.lock().unwrap().stop(id);
}

/// Moves the files of the last move back. `id` numbers it for `cancel_sort`.
#[tauri::command]
async fn undo_sort(
    app: AppHandle,
    state: State<'_, AppState>,
    id: u64,
    on_progress: Channel<sort::Progress>,
) -> Result<sort::Undone, String> {
    let mut journal = state.journal.lock().unwrap().take().ok_or("There's nothing to undo.")?;
    let root = state.root.lock().unwrap().clone().unwrap_or_default();
    let cancel = state.work.lock().unwrap().start(id);
    let mut reporter = Reporter::new(&app, on_progress);
    state.working.store(true, Ordering::SeqCst);
    let undone = tauri::async_runtime::spawn_blocking(move || {
        let undone = sort::undo(&mut journal, &root, &cancel, |progress| reporter.report(progress));
        reporter.finish();
        (undone, journal)
    })
    .await;
    finish_work(&app, &state);
    let (undone, journal) = undone.map_err(describe)?;
    state.undone(&undone, journal);
    Ok(undone)
}

/// Free space where the clips would go: `destination`, or the scanned folder.
#[tauri::command]
async fn disk_space(state: State<'_, AppState>, destination: Option<PathBuf>) -> Result<DiskSpace, String> {
    let root = state.root.lock().unwrap().clone().ok_or("Choose a folder first.")?;
    // A network drive can take a moment to answer, so this stays off the main thread.
    tauri::async_runtime::spawn_blocking(move || {
        let target = destination.unwrap_or_else(|| root.clone());
        let free = sort::free_space(&target)?;
        Ok(DiskSpace { free, same_drive: sort::same_drive(&root, &target)? })
    })
    .await
    .map_err(describe)?
    .map_err(|e: io::Error| describe(e))
}

/// The folder D-Sep.exe is in, offered on the start screen. None when installed: that folder holds no clips.
#[tauri::command]
fn exe_folder() -> Option<PathBuf> {
    // The installer stamps its bundle type into the exe; the portable exe has none.
    if tauri::utils::platform::bundle_type().is_some() {
        return None;
    }
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// Opens one of the folders the UI shows: the scanned folder, or one the last sort put clips in.
#[tauri::command]
fn open_folder(state: State<'_, AppState>, path: PathBuf) -> Result<(), String> {
    if !state.openable.lock().unwrap().contains(&path) {
        return Err("D-Sep only opens the folders it scanned or sorted into.".into());
    }
    if !path.is_dir() {
        return Err("That folder no longer exists.".into());
    }
    explore(path.as_os_str())
}

/// Opens the GitHub page in the default browser. The URL is fixed, so the UI can't open arbitrary ones.
#[tauri::command]
fn open_repo() -> Result<(), String> {
    explore(OsStr::new(env!("CARGO_PKG_REPOSITORY")))
}

/// Passes progress to the UI about 30 times a second, and whenever the file changes, and shows it on the
/// taskbar button.
struct Reporter {
    channel: Channel<sort::Progress>,
    window: Option<WebviewWindow>,
    last: Option<(usize, Instant)>,
    percent: Option<u64>,
}

impl Reporter {
    fn new(app: &AppHandle, channel: Channel<sort::Progress>) -> Self {
        Reporter { channel, window: app.get_webview_window("main"), last: None, percent: None }
    }

    fn report(&mut self, progress: &sort::Progress) {
        let due = match self.last {
            Some((index, at)) => {
                index != progress.index
                    || progress.bytes_done == progress.bytes_total
                    || at.elapsed() >= Duration::from_millis(33)
            }
            None => true,
        };
        if !due {
            return;
        }
        self.last = Some((progress.index, Instant::now()));
        let _ = self.channel.send(progress.clone());
        let percent = (progress.bytes_done * 100).checked_div(progress.bytes_total).unwrap_or(0).min(100);
        if self.percent != Some(percent) {
            self.percent = Some(percent);
            if let Some(window) = &self.window {
                let _ = window.set_progress_bar(ProgressBarState {
                    status: Some(ProgressBarStatus::Normal),
                    progress: Some(percent),
                });
            }
        }
    }

    /// Clears the taskbar button, and flashes it when D-Sep isn't the window in front.
    fn finish(&self) {
        if let Some(window) = &self.window {
            let _ = window.set_progress_bar(ProgressBarState { status: Some(ProgressBarStatus::None), progress: None });
            if !window.is_focused().unwrap_or(true) {
                let _ = window.request_user_attention(Some(UserAttentionType::Informational));
            }
        }
    }
}

/// Ends a sort or an undo, and quits if "Stop and quit" was chosen meanwhile.
fn finish_work(app: &AppHandle, state: &AppState) {
    state.working.store(false, Ordering::SeqCst);
    if state.quit.load(Ordering::SeqCst) {
        app.exit(0);
    }
}

/// Asks whether to stop and quit. Stopping cancels the work, and `finish_work` quits once it has stopped.
fn ask_to_stop(window: &tauri::Window) {
    use rfd::{AsyncMessageDialog, MessageButtons, MessageDialogResult, MessageLevel};
    const STOP: &str = "Stop and quit";
    // rfd itself, not the dialog plugin: "Keep going" comes first so that Enter keeps going, and the plugin would
    // report Escape as the second button, "Stop and quit", where rfd reports Cancel.
    let answer = AsyncMessageDialog::new()
        .set_title("Stop and quit?")
        .set_description("Files that aren't done yet are left as they were.")
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::OkCancelCustom("Keep going".into(), STOP.into()))
        .set_parent(window)
        .show();
    let app = window.app_handle().clone();
    std::thread::spawn(move || {
        let stop = tauri::async_runtime::block_on(answer) == MessageDialogResult::Custom(STOP.into());
        let state = app.state::<AppState>();
        state.asking.store(false, Ordering::SeqCst);
        if stop {
            // `quit` before `working` is read, so work that's just ending still sees it.
            state.quit.store(true, Ordering::SeqCst);
            state.work.lock().unwrap().cancel.store(true, Ordering::SeqCst);
            if !state.working.load(Ordering::SeqCst) {
                app.exit(0);
            }
        }
    });
}

/// Opens a folder, or a URL in the default browser, with Explorer.
fn explore(target: &OsStr) -> Result<(), String> {
    explorer(target).and_then(|mut command| command.spawn()).map(drop).map_err(describe)
}

/// Explorer by its full path, started from the Windows folder, with `target` in quotes. A bare "explorer" is looked
/// for next to D-Sep.exe first, so any explorer.exe that sits there (say, in Downloads) would run instead. And std
/// quotes an argument only when it has a space, while Explorer splits an unquoted one at each comma and opens, or
/// runs, what follows: "E:\Clips,setup.exe" would run a setup.exe in the folder D-Sep was started from. A path
/// can't contain a quote.
#[cfg(windows)]
fn explorer(target: &OsStr) -> io::Result<Command> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
    let mut buf = [0; 260];
    // SAFETY: the length passed is the buffer's.
    let len = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len >= buf.len() {
        return Err(io::Error::other("Can't find the Windows folder."));
    }
    let windows = PathBuf::from(OsString::from_wide(&buf[..len]));
    let mut quoted = OsString::from("\"");
    quoted.push(target);
    quoted.push("\"");
    let mut command = Command::new(windows.join("explorer.exe"));
    command.raw_arg(quoted).current_dir(windows);
    Ok(command)
}

#[cfg(not(windows))]
fn explorer(target: &OsStr) -> io::Result<Command> {
    let mut command = Command::new("explorer");
    command.arg(target);
    Ok(command)
}

/// An error message without the trailing "(os error 3)".
fn describe(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    match message.rfind(" (os error ") {
        Some(at) => message[..at].to_owned(),
        None => message,
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // The window from tauri.conf.json exists by now. Setting the title here needs no capability.
            if let Some(window) = app.get_webview_window("main") {
                let info = app.package_info();
                window.set_title(&format!("{} v{}", info.name, info.version))?;
                // It starts hidden, and the UI shows it once the saved theme is on, so it never opens light and
                // turns dark. Should the UI not get that far, it's shown anyway.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(3));
                    if !window.is_visible().unwrap_or(true) {
                        let _ = window.show();
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.working.load(Ordering::SeqCst) {
                    // Quitting now would cut a copy short, so ask first.
                    api.prevent_close();
                    if !state.asking.swap(true, Ordering::SeqCst) {
                        ask_to_stop(window);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            scan_folder,
            cancel_scan,
            in_place,
            sort_clips,
            cancel_sort,
            undo_sort,
            disk_space,
            exe_folder,
            open_folder,
            open_repo
        ])
        .run(tauri::generate_context!())
        .expect("error while running D-Sep");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_drops_the_os_error_code() {
        let error = std::io::Error::from_raw_os_error(3);
        let full = error.to_string();
        assert!(full.ends_with(" (os error 3)"), "{full:?}");
        assert_eq!(describe(&error), full.trim_end_matches(" (os error 3)"));
        assert_eq!(describe("That folder no longer exists."), "That folder no longer exists.");
    }

    #[test]
    fn a_cancel_stops_only_the_work_it_was_meant_for() {
        let mut work = Work::default();
        // A Cancel that arrives before its sort has started still stops it.
        work.stop(1);
        assert!(work.start(1).load(Ordering::SeqCst));
        // One that arrives after its sort has ended doesn't stop the next.
        let undo = work.start(2);
        work.stop(1);
        assert!(!undo.load(Ordering::SeqCst));
        work.stop(2);
        assert!(undo.load(Ordering::SeqCst));
    }

    #[test]
    fn keeps_a_move_for_undo_until_the_next_scan() {
        let state = AppState::default();
        let root = PathBuf::from(r"E:\DCIM");
        let scan = Scan { root: root.clone(), clips: Vec::new() };
        let journal = |state: &AppState| state.journal.lock().unwrap().is_some();
        let openable = |state: &AppState| state.openable.lock().unwrap().clone();
        state.scanned(&scan);
        assert_eq!(openable(&state), std::slice::from_ref(&root));

        // A copy has nothing to undo; a move does. Either way, the folders its clips went to may be opened.
        let into = PathBuf::from(r"F:\Project");
        let folders = sort::ProfileFolders { dlog: Some(into.join("D-Log")), ..Default::default() };
        let copied = sort::Summary { folder: into.clone(), folders, ..Default::default() };
        state.sorted(&copied, sort::Journal::default());
        assert!(!journal(&state));
        assert_eq!(openable(&state), [root.clone(), into.clone(), into.join("D-Log")]);
        state.sorted(&sort::Summary { undoable: true, ..copied }, sort::Journal::default());
        assert!(journal(&state));

        // An undo takes the journal, and gives it back only when files remain to put back.
        let taken = state.journal.lock().unwrap().take().unwrap();
        state.undone(&sort::Undone { remaining: 1, ..Default::default() }, taken);
        assert!(journal(&state));
        let taken = state.journal.lock().unwrap().take().unwrap();
        state.undone(&sort::Undone::default(), taken);
        assert!(!journal(&state));

        // A new scan drops the last move's undo, and the folders the sort opened.
        state.sorted(&sort::Summary { undoable: true, folder: into, ..Default::default() }, sort::Journal::default());
        state.scanned(&scan);
        assert!(!journal(&state));
        assert_eq!(openable(&state), [root]);
    }

    #[cfg(windows)]
    #[test]
    fn quotes_what_explorer_opens() {
        let command = explorer(OsStr::new(r"E:\Clips,setup.exe")).unwrap();
        assert_eq!(command.get_args().collect::<Vec<_>>(), [OsStr::new(r#""E:\Clips,setup.exe""#)]);
        // Started from the Windows folder, where nothing a card brings can be found.
        let windows = command.get_current_dir().unwrap();
        assert_eq!(command.get_program(), windows.join("explorer.exe"));
    }
}
