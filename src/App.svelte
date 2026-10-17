<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getVersion } from "@tauri-apps/api/app";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import logo from "./assets/logo.svg";
  import {
    cancelScan,
    cancelSort,
    diskSpace,
    exeFolder,
    inPlace,
    openFolder,
    openRepo,
    scanFolder,
    sortClips,
    undoSort,
    type Action,
    type Clip,
    type DiskSpace,
    type Layout,
    type Mode,
    type Progress,
    type Summary,
  } from "./lib/api";
  import {
    dropNotice,
    extension,
    formatBytes,
    formatDuration,
    joinPath,
    plural,
    relativeTo,
    sameFolder,
    splitPath,
  } from "./lib/format";
  import {
    DEFAULT_PREFIXES,
    MAX_PREFIX,
    cleanPrefix,
    isDevicePrefix,
    loadOptions,
    saveOptions,
    type Theme,
  } from "./lib/options";

  type Phase = "idle" | "scanning" | "ready" | "sorting" | "undoing" | "done";
  type Sheet = "about" | "help" | "licenses";
  type LogKind = "dlog" | "dlog2" | "dlogm";
  /** A scan under way: its folder, and how far it's got. */
  type Scanning = { path: string; found: number; probed: number };
  /** The options of the last sort, for its result screen. */
  type Sorted = { mode: Mode; action: Action; destination: string | null; verify: boolean };

  const LOG_KINDS: LogKind[] = ["dlog", "dlog2", "dlogm"];
  const FOLDERS: Record<LogKind, string> = { dlog: "D-Log", dlog2: "D-Log 2", dlogm: "D-Log M" };
  const THEMES: { theme: Theme; name: string; title: string }[] = [
    { theme: "light", name: "Light", title: "Always light" },
    { theme: "dark", name: "Dark", title: "Always dark" },
    { theme: "auto", name: "Auto", title: "Follow Windows" },
  ];
  // Icons are stroked paths on a 24×24 grid.
  const CHOICES: { action: Action; title: string; icon: string }[] = [
    {
      action: "folders",
      title: "Folders",
      icon: "M3 7.5A2.5 2.5 0 0 1 5.5 5h3.6a2 2 0 0 1 1.4.6L12 7h6.5A2.5 2.5 0 0 1 21 9.5v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.5z",
    },
    { action: "rename", title: "Names", icon: "M5 19l1-4L15.5 5.5a2.12 2.12 0 0 1 3 3L9 18zM13.5 7.5l3 3" },
    {
      action: "both",
      title: "Folders + Names",
      // The folder, open at its bottom-left corner, with a short pencil across it.
      icon: "M3 11.5v-4A2.5 2.5 0 0 1 5.5 5h3.6a2 2 0 0 1 1.4.6L12 7h6.5A2.5 2.5 0 0 1 21 9.5v7a2.5 2.5 0 0 1-2.5 2.5h-8M2 20l1-4 6-6a2.12 2.12 0 0 1 3 3l-6 6zM7 12l3 3",
    },
  ];
  const MODES: { mode: Mode; name: string }[] = [
    { mode: "copy", name: "Copy" },
    { mode: "move", name: "Move" },
  ];
  const LAYOUTS: { layout: Layout; name: string; title: string }[] = [
    { layout: "gather", name: "Together", title: "One set of D-Log folders for this folder and all its subfolders" },
    { layout: "each", name: "Per folder", title: "D-Log folders inside each subfolder, next to its clips" },
  ];
  // Renaming in place is a move that stays in the folder, and instant, so it shows no size.
  const VERBS = {
    copy: { name: "Copy", doing: "Copying", done: "copied", did: "Copied" },
    move: { name: "Move", doing: "Moving", done: "moved", did: "Moved" },
    rename: { name: "Rename", doing: "Renaming", done: "renamed", did: "Renamed" },
    undo: { name: "Undo", doing: "Moving back", done: "moved back", did: "Moved back" },
  };
  const SUBFOLDERS_HELP = "Also look for clips in the folders inside this one";
  const SIDECARS_HELP = "Proxy and telemetry files with the same name as a clip";
  const VERIFY_HELP = "Read each copy back from the disk and compare it with its original. Takes about twice as long.";
  const NO_VERIFY_HELP = "A move within a drive copies nothing, so there's nothing to verify.";
  const saved = loadOptions();

  let phase = $state<Phase>("idle");
  let root = $state("");
  let clips = $state.raw<Clip[]>([]);
  let error = $state("");
  /** News that isn't a problem, like what an undo did. */
  let notice = $state("");
  let dragging = $state(false);
  let scanning = $state.raw<Scanning | null>(null);
  let progress = $state.raw<Progress | null>(null);
  /** Bytes per second over the last few seconds; 0 until there's enough to tell. */
  let rate = $state(0);
  let summary = $state.raw<Summary | null>(null);
  let sorted = $state.raw<Sorted>({ mode: "copy", action: "folders", destination: null, verify: false });
  let stopping = $state(false);
  /** The paths of the log clips a sort would leave alone, already in place. */
  let placedAlready = $state.raw<Set<string>>(new Set());
  /** Why what was just typed into a prefix vanished, until the next keystroke that keeps. */
  let prefixNote = $state("");
  /** The third-party licenses, read when their sheet first opens. */
  let licenses = $state("");
  let space = $state.raw<DiskSpace | null>(null);
  /** The folder D-Sep.exe is in, offered on the start screen; null for an installed D-Sep. */
  let here = $state<string | null>(null);
  let menuOpen = $state(false);
  // What the dialog shows. It isn't cleared on close: a late "close" event would then empty the next sheet.
  let sheet = $state<Sheet | null>(null);
  let version = $state("");
  let menu: HTMLDivElement;
  let cog: HTMLButtonElement;
  let dialog: HTMLDialogElement;
  let list: HTMLDivElement | undefined = $state();
  let resultHeading: HTMLHeadingElement | undefined = $state();

  /** Where the clips go, when not into the scanned folder. Not saved: each session starts with the folder itself. */
  let destination = $state<string | null>(null);
  let mode = $state<Mode>(saved.mode);
  let action = $state<Action>(saved.action);
  let prefixes = $state(saved.prefixes);
  let recursive = $state(saved.recursive);
  let sidecars = $state(saved.sidecars);
  let theme = $state<Theme>(saved.theme);
  let layout = $state<Layout>(saved.layout);
  let verify = $state(saved.verify);

  /** Numbers the scans, so a Cancel stops the one it was meant for. */
  let scanId = 0;
  /** Numbers the sorts and undos, so a Cancel that comes late can't stop the next one. */
  let workId = 0;
  /** Recent progress readings, for the rate. */
  let readings: { at: number; bytes: number }[] = [];
  /** Numbers the free-space requests, so a late answer can't replace a newer one. */
  let spaceRequest = 0;
  /** Numbers the already-in-place requests, for the same reason. */
  let placedRequest = 0;
  /** The window starts hidden, and is shown once, when the saved theme is on. */
  let shown = false;

  const logClips = $derived(clips.filter((c) => isLog(c.profile.kind)));
  const clipsOf = $derived.by(() => {
    const of = (kind: LogKind) => logClips.filter((c) => c.profile.kind === kind);
    return { dlog: of("dlog"), dlog2: of("dlog2"), dlogm: of("dlogm") };
  });
  const otherCount = $derived(clips.length - logClips.length);
  const unreadable = $derived(clips.filter((c) => c.error).length);
  const sidecarKinds = $derived(
    [...new Set(logClips.flatMap((c) => c.sidecars.map((s) => extension(s.path))))].sort().join(", "),
  );
  /** The log clips a sort would work on: those not already in place. */
  const toSort = $derived(logClips.filter((c) => !placedAlready.has(c.path)));
  const sortBytes = $derived(bytes(toSort, sidecars));
  // Position of each log clip, by name, in the sort queue, which follows list order.
  const queue = $derived(new Map(logClips.map((c, i) => [c.name, i])));
  /** The folder on show: the one being scanned, or the last one scanned. */
  const folder = $derived(splitPath(scanning ? scanning.path : root));
  const busy = $derived(phase === "scanning" || phase === "sorting" || phase === "undoing");
  const working = $derived(phase === "sorting" || phase === "undoing");
  const percent = $derived(
    progress && progress.bytesTotal > 0 ? Math.min(100, (progress.bytesDone / progress.bytesTotal) * 100) : 0,
  );
  const placed = $derived(summary ? summary.dlog + summary.dlog2 + summary.dlogm : 0);
  const clean = $derived(!!summary && !summary.cancelled && summary.failed.length === 0);
  const verb = $derived(verbFor(action, mode, destination));
  const workVerb = $derived(phase === "undoing" ? "undo" : verbFor(sorted.action, sorted.mode, sorted.destination));
  /** Some log clips are in subfolders, so where their folders go is a choice. */
  const nested = $derived(logClips.some((c) => /[\\/]/.test(c.name)));
  /** A move to another drive is a copy, and takes the time and the space one does. */
  const copies = $derived(mode === "copy" || (!!space && !space.sameDrive));
  const needed = $derived(copies ? sortBytes : 0);
  const short = $derived(!!space && needed > space.free);
  // Renames within a drive are instant, so a rate means nothing for them.
  const showRate = $derived(
    working && rate > 0 && ((phase === "sorting" && sorted.mode === "copy") || (!!space && !space.sameDrive)),
  );
  const timeLeft = $derived(progress && rate > 0 ? (progress.bytesTotal - progress.bytesDone) / rate : 0);
  // Log clips to sort whose names already start with their prefix, so they keep them.
  const keeping = $derived(action === "folders" ? 0 : toSort.filter((c) => !newPrefix(c)).length);
  /** What's wrong with the prefixes, and whose they are, so those fields can be marked. */
  const prefixCheck = $derived.by((): { problem: string; bad: LogKind[] } => {
    if (action === "folders") return { problem: "", bad: [] };
    const missing = LOG_KINDS.find((kind) => !prefixes[kind]);
    if (missing) return { problem: `Type a prefix for ${FOLDERS[missing]}.`, bad: [missing] };
    const device = LOG_KINDS.find((kind) => isDevicePrefix(prefixes[kind]));
    if (device) {
      return { problem: `The ${FOLDERS[device]} prefix can't start with a device name like CON or NUL.`, bad: [device] };
    }
    if (action !== "rename") return { problem: "", bad: [] };
    for (const [i, a] of LOG_KINDS.entries()) {
      const b = LOG_KINDS.slice(i + 1).find((kind) => prefixes[kind].toLowerCase() === prefixes[a].toLowerCase());
      if (b) return { problem: `Use different prefixes, or ${FOLDERS[a]} and ${FOLDERS[b]} clips will look the same.`, bad: [a, b] };
    }
    return { problem: "", bad: [] };
  });
  const prefixProblem = $derived(prefixCheck.problem);
  /** The profile folders the clips go into, named: only the profiles there are clips of. */
  const intoFolders = $derived.by(() => {
    const present = LOG_KINDS.filter((kind) => clipsOf[kind].length).map((kind) => FOLDERS[kind]);
    const names = present.length ? present : Object.values(FOLDERS);
    const listed = names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
    return { listed, one: names.length === 1 };
  });
  /** What pressing the button will do, in a sentence. */
  const plan = $derived.by(() => {
    const done = mode === "copy" ? "copied" : "moved";
    const into = destination ? splitPath(destination).name : "";
    if (action === "rename") {
      if (destination) return `Clips will be ${done} to ${into} with new names, in the same subfolders.`;
      return mode === "copy" ? "Clips will be copied with new names, next to the originals." : "Clips will be renamed where they are.";
    }
    const renamed = action === "both" ? " and renamed" : "";
    const { listed, one } = intoFolders;
    if (layout === "each" && nested) {
      const where = destination ? `, in the same subfolders of ${into}` : "";
      return `Clips will be ${done}${renamed} into ${listed} folders next to them${where}.`;
    }
    const where = destination ? `in ${into}` : "in this folder";
    const gathered = nested ? " Clips from subfolders go there too." : "";
    return `Clips will be ${done}${renamed} into ${one ? `a ${listed} folder` : `${listed} folders`} ${where}.${gathered}`;
  });
  /** What the progress panel says is under way. */
  const doing = $derived(progress?.verifying ? "Verifying" : VERBS[workVerb].doing);
  // Read out once per file, and again when its copy is being verified; the bytes change too often to announce.
  const announcement = $derived(working && progress && progress.index ? `${doing} ${progress.index} of ${progress.count}` : "");
  /** The queue position of the clip being sorted, or -1. Found by name, as the sort skips clips already in place. */
  const activeAt = $derived(phase === "sorting" && progress?.file ? (queue.get(progress.file) ?? -1) : -1);

  $effect(() => {
    saveOptions({ mode, action, recursive, sidecars, prefixes, theme, layout, verify });
  });

  // The window's theme colors its title bar and sets WebView2's prefers-color-scheme, which app.css follows. The
  // window starts hidden (tauri.conf.json) and is shown once the saved theme is on, so it never opens light and
  // then turns dark.
  $effect(() => {
    const dark = theme === "auto" ? null : theme === "dark";
    getCurrentWindow()
      .setTheme(theme === "auto" ? null : theme)
      .catch(() => {})
      .then(async () => {
        if (shown) return;
        shown = true;
        if (dark !== null) await schemeIs(dark);
        await getCurrentWindow().show().catch(() => {});
      });
  });

  // Which clips are already in place, for their marks in the list and the count on the button. Asked again a moment
  // after the options that decide where clips go stop changing.
  $effect(() => {
    const request = ++placedRequest;
    // Kept while sorting, which skips those clips: the list still shows why it counts fewer.
    if (phase === "sorting") return;
    if (phase !== "ready" || prefixProblem) {
      placedAlready = new Set();
      return;
    }
    // Copy or move makes no difference to where clips go, so it's left out and doesn't ask again.
    const options = { mode: "copy" as const, action, sidecars, prefixes: { ...prefixes }, destination, layout, verify: false };
    const timer = setTimeout(() => {
      inPlace(options).then(
        (paths) => request === placedRequest && (placedAlready = new Set(paths)),
        () => request === placedRequest && (placedAlready = new Set()),
      );
    }, 150);
    return () => clearTimeout(timer);
  });

  // Free space where the clips would go, for the warning before a copy.
  $effect(() => {
    if (phase !== "ready") return;
    const request = ++spaceRequest;
    diskSpace(destination).then(
      (result) => request === spaceRequest && (space = result),
      () => request === spaceRequest && (space = null),
    );
  });

  // Keep the clip being sorted in view.
  $effect(() => {
    if (activeAt < 0) return;
    tick().then(() => list?.querySelector(".row.active")?.scrollIntoView({ block: "nearest" }));
  });

  onMount(() => {
    const unlisten = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "drop") {
        dragging = false;
        const paths = payload.paths;
        if (!busy && paths.length) {
          // What was dropped is clear once the folder is: files show all of theirs, and of folders only the first.
          scan(paths[0]).then((scanned) => scanned && (notice = dropNotice(paths, root)));
        }
      } else {
        dragging = payload.type !== "leave" && !busy;
        // Make way for the drop feedback, so a drop doesn't start a scan hidden behind the menu or a sheet.
        if (dragging) {
          menuOpen = false;
          dialog.close();
        }
      }
    });
    // Without them, About shows no version and the start screen no folder of D-Sep's own.
    getVersion().then((value) => (version = value), () => {});
    exeFolder().then((path) => (here = path), () => {});
    return () => {
      unlisten.then((stop) => stop());
    };
  });

  function editPrefix(kind: LogKind, input: HTMLInputElement) {
    const value = cleanPrefix(input.value);
    // What a prefix can't have is dropped as it's typed. Say why, or it just seems to vanish.
    prefixNote =
      value === input.value
        ? ""
        : `A prefix can't have \\ / : * ? " < > |, start with a dot, $ or a space, or be longer than ${MAX_PREFIX} characters.`;
    if (value !== input.value) input.value = value;
    prefixes[kind] = value;
  }

  /** Waits, up to half a second, for WebView2 to report the color scheme the window was just given. */
  async function schemeIs(dark: boolean) {
    const query = matchMedia("(prefers-color-scheme: dark)");
    for (let waited = 0; query.matches !== dark && waited < 500; waited += 25) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }

  function isLog(kind: Clip["profile"]["kind"]): kind is LogKind {
    return (LOG_KINDS as string[]).includes(kind);
  }

  /** Renaming is a move that stays in the clip's folder; into another folder, it's a move. */
  function verbFor(action: Action, mode: Mode, destination: string | null): "copy" | "move" | "rename" {
    return mode === "copy" ? "copy" : action === "rename" && !destination ? "rename" : "move";
  }

  /** "Copied 5 clips into folders in", "Renamed 5 clips in", …: the folder's name follows. */
  function resultLine(count: number, { action, mode, destination }: Sorted): string {
    const verb = verbFor(action, mode, destination);
    const clips = plural(count, "clip");
    if (verb === "rename") return `Renamed ${clips} in`;
    if (action === "rename") return `${VERBS[verb].did} ${clips} with new names ${destination ? "to" : "in"}`;
    return `${VERBS[verb].did}${action === "both" ? " and renamed" : ""} ${clips} into folders in`;
  }

  /** The note for clips that were already where they'd go. */
  function existingNote(count: number): string {
    const clips = plural(count, "clip");
    const one = count === 1;
    const skipped = `so ${one ? "it was" : "they were"} skipped`;
    if (sorted.action === "rename" && !sorted.destination) return `${clips} already had ${one ? "its" : "their"} prefix, ${skipped}.`;
    // Same size and date is what tells a copy, not the contents, so the note doesn't claim they're identical.
    const already = `${clips} ${one ? "was" : "were"} already in place (same size and date)`;
    if (sorted.mode === "copy") return `${already}, ${skipped}.`;
    return `${already}, so the original${one ? " stays where it is" : "s stay where they are"}.`;
  }

  /** The prefix a log clip will get in front of its name, or "" when it keeps it. */
  function newPrefix(clip: Clip): string {
    const kind = clip.profile.kind;
    if (action === "folders" || !isLog(kind)) return "";
    const prefix = prefixes[kind];
    return splitPath(clip.name).name.toLowerCase().startsWith(prefix.toLowerCase()) ? "" : prefix;
  }

  /** The folder a log clip goes to. Mirrors Job::new in sort.rs. */
  function targetDir(clip: Clip, kind: LogKind): string {
    const base = destination ?? root;
    // A clip already inside a destination that's inside the scanned folder keeps its place there.
    const inside = relativeTo(base, root) === null ? null : relativeTo(splitPath(clip.path).parent, base);
    const sub = inside ?? splitPath(clip.name).parent;
    const own = sub ? joinPath(base, sub) : base;
    if (action === "rename") return own;
    return outputDir(layout === "each" ? own : base, FOLDERS[kind]);
  }

  /** The profile's folder inside `dir`, or beside it when `dir` is one of them. Mirrors output_dir in sort.rs. */
  function outputDir(dir: string, name: string): string {
    const own = splitPath(dir);
    const is = (folder: string) => own.name.toLowerCase() === folder.toLowerCase();
    if (is(name)) return dir;
    return Object.values(FOLDERS).some(is) ? joinPath(own.parent, name) : joinPath(dir, name);
  }

  /** Where a log clip will end up, relative to where the clips go when it's inside; "" for clips that stay. */
  function targetOf(clip: Clip): string {
    const kind = clip.profile.kind;
    if (!isLog(kind)) return "";
    const path = joinPath(targetDir(clip, kind), newPrefix(clip) + splitPath(clip.name).name);
    const base = joinPath(destination ?? root, "");
    return path.toLowerCase().startsWith(base.toLowerCase()) ? path.slice(base.length) : path;
  }

  function bytes(list: Clip[], withSidecars: boolean): number {
    return list.reduce(
      (sum, clip) => sum + clip.size + (withSidecars ? clip.sidecars.reduce((s, file) => s + file.size, 0) : 0),
      0,
    );
  }

  /** Scans `path` and shows it. Cancelled or failed, the previous view comes back. Returns whether it worked. */
  async function scan(path: string, note = ""): Promise<boolean> {
    const before = { phase, clips, summary };
    const focused = document.activeElement;
    const id = ++scanId;
    phase = "scanning";
    error = "";
    notice = note;
    scanning = { path, found: 0, probed: 0 };
    clips = [];
    try {
      const result = await scanFolder(path, recursive, id, (update) => (scanning = { path, ...update }));
      if (result) {
        root = result.root;
        clips = result.clips;
        summary = null;
        if (destination && sameFolder(destination, root)) destination = null;
        phase = "ready";
        restoreFocus(focused, () => list);
        return true;
      }
      notice = "";
    } catch (e) {
      error = String(e);
    } finally {
      scanning = null;
    }
    phase = before.phase;
    clips = before.clips;
    summary = before.summary;
    return false;
  }

  function stopScan() {
    cancelScan(scanId);
  }

  async function browse() {
    if (busy) return;
    const picked = await open({ directory: true, title: "Choose a folder with DJI clips", defaultPath: root || undefined });
    if (typeof picked === "string") await scan(picked);
  }

  async function chooseDestination() {
    const picked = await open({ directory: true, title: "Choose where the clips go", defaultPath: destination ?? root });
    if (typeof picked === "string") destination = sameFolder(picked, root) ? null : picked;
  }

  async function setRecursive(value: boolean) {
    const was = recursive;
    recursive = value;
    // A cancelled rescan keeps the old list, so the checkbox goes back to match it.
    if (root && !(await scan(root))) recursive = was;
  }

  /** Shows a progress update, and works out the rate from the last few seconds of them. */
  function track(update: Progress) {
    progress = update;
    const at = performance.now();
    readings.push({ at, bytes: update.bytesDone });
    while (readings.length > 2 && at - readings[0].at > 4000) readings.shift();
    const first = readings[0];
    rate = at - first.at > 1000 ? ((update.bytesDone - first.bytes) * 1000) / (at - first.at) : 0;
  }

  function beginWork(count: number, bytesTotal: number) {
    stopping = false;
    error = "";
    notice = "";
    readings = [];
    rate = 0;
    progress = { index: 0, count, file: "", bytesDone: 0, bytesTotal, verifying: false };
  }

  async function start() {
    const id = ++workId;
    // Only a copy has anything to verify; a move within a drive is a rename.
    const options = { mode, action, sidecars, prefixes: { ...prefixes }, destination, layout, verify: verify && copies };
    sorted = { mode, action, destination, verify: options.verify };
    const focused = document.activeElement;
    beginWork(toSort.length, sortBytes * (options.verify ? 2 : 1));
    phase = "sorting";
    try {
      summary = await sortClips(options, id, track);
      phase = "done";
      restoreFocus(focused, () => resultHeading);
    } catch (e) {
      error = String(e);
      phase = "ready";
    }
  }

  function stop() {
    stopping = true;
    cancelSort(workId);
  }

  async function undo() {
    const id = ++workId;
    beginWork(0, 0);
    phase = "undoing";
    try {
      const result = await undoSort(id, track);
      const moved = `Moved ${plural(result.restored, "file")} back.`;
      const failed = result.failed.length
        ? `Couldn't move back ${plural(result.failed.length, "file")}: ` +
          result.failed.map((f) => `${f.file} (${f.reason})`).join("; ")
        : "";
      if (result.remaining) {
        // Some files are still where the sort put them, so Undo stays on offer.
        phase = "done";
        notice = result.cancelled ? `Stopped. ${moved}` : moved;
        error = failed;
        restoreFocus(null, () => resultHeading);
      } else {
        // Everything is back, so a cancelled rescan returns to a result screen without Undo, and says what the undo
        // did, rather than to the undo's own progress.
        phase = "done";
        if (summary) summary = { ...summary, undoable: false };
        if (!(await scan(root, moved))) notice = moved;
        if (failed) error = failed;
      }
    } catch (e) {
      error = String(e);
      phase = "done";
    }
  }

  /**
   * Once the view has changed, puts focus back where it was if that's still there and can take it, or on
   * `fallback`, so that keyboard users don't land back at the top of the window.
   */
  async function restoreFocus(was: Element | null, fallback: () => HTMLElement | undefined) {
    await tick();
    if (was instanceof HTMLElement && was !== document.body && was.isConnected && !was.matches(":disabled")) {
      was.focus();
    } else {
      fallback()?.focus();
    }
  }

  async function reveal(path: string) {
    try {
      await openFolder(path);
    } catch (e) {
      error = String(e);
    }
  }

  /** Closes the menu and puts focus back on the cog, where a closing sheet then returns it. */
  function closeMenu() {
    menuOpen = false;
    cog.focus();
  }

  /** Arrow keys, Home and End move through the menu, theme buttons included; Escape and Tab leave it. */
  function menuKeys(event: KeyboardEvent & { currentTarget: HTMLElement }) {
    const items = [...event.currentTarget.querySelectorAll("button")];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    let next: number;
    switch (event.key) {
      case "ArrowDown":
      case "ArrowRight":
        next = at + 1;
        break;
      case "ArrowUp":
      case "ArrowLeft":
        next = at - 1;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = -1;
        break;
      case "Escape":
        event.preventDefault();
        closeMenu();
        return;
      case "Tab":
        // Tab carries on from the cog.
        closeMenu();
        return;
      default:
        return;
    }
    event.preventDefault();
    items.at(next % items.length)?.focus();
  }

  /**
   * Arrow keys, Home and End move the choice in a radio group, and focus with it, as ARIA radio groups do.
   * Only the chosen radio is a Tab stop.
   */
  function radioKeys<T>(
    event: KeyboardEvent & { currentTarget: HTMLElement },
    values: readonly T[],
    current: T,
    choose: (value: T) => void,
  ) {
    const steps: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };
    const at = values.indexOf(current);
    let next: number;
    if (event.key === "Home") next = 0;
    else if (event.key === "End") next = values.length - 1;
    else if (event.key in steps) next = (at + steps[event.key] + values.length) % values.length;
    else return;
    event.preventDefault();
    choose(values[next]);
    const group = event.currentTarget;
    tick().then(() => group.querySelector<HTMLElement>('[aria-checked="true"]')?.focus());
  }

  async function showSheet(which: Sheet) {
    closeMenu();
    sheet = which;
    if (which === "licenses" && !licenses) {
      fetch("/third-party-notices.txt")
        .then((response) => (response.ok ? response.text() : Promise.reject()))
        .then(
          (text) => (licenses = text),
          () => (licenses = "The licenses couldn't be read."),
        );
    }
    await tick();
    // From About, the dialog is already open: focus goes to its close button, as the button pressed is gone.
    if (dialog.open) dialog.querySelector<HTMLElement>(".close")?.focus();
    else dialog.showModal();
  }

  async function visitRepo() {
    closeMenu();
    try {
      await openRepo();
    } catch (e) {
      error = String(e);
    }
  }

  // Reads activeAt rather than progress, so rows update once per file, not with every progress update.
  function rowState(clip: Clip): string {
    const at = queue.get(clip.name);
    if (activeAt < 0 || at === undefined) return "";
    if (at < activeAt) return "done";
    return at === activeAt ? "active" : "";
  }

  function badge(clip: Clip): { text: string; tone: string; title: string } {
    if (clip.error) return { text: "Unreadable", tone: "error", title: clip.error };
    switch (clip.profile.kind) {
      case "dlog":
      case "dlog2":
      case "dlogm": {
        const where = placedAlready.has(clip.path) ? "Already sorted:" : "Goes to";
        return { text: FOLDERS[clip.profile.kind], tone: clip.profile.kind, title: `${where} ${targetOf(clip)}` };
      }
      case "other":
        return { text: clip.profile.label, tone: "", title: "Not D-Log, D-Log 2 or D-Log M, left untouched" };
      case "unknown":
        return { text: "No profile", tone: "", title: "No DJI color profile in this file, left untouched" };
    }
  }
</script>

<svelte:window
  onpointerdown={(e) => {
    if (menuOpen && !menu.contains(e.target as Node)) menuOpen = false;
  }}
  onblur={() => (menuOpen = false)}
/>

{#snippet description()}
  Sort DJI clips shot in <span>D-Log</span>, <span>D-Log 2</span> and <span>D-Log M</span>.
{/snippet}

<!-- Hovering shows the same text as the title around it; a click shows it at once. -->
{#snippet help(id: string, text: string)}
  <button type="button" class="help" popovertarget={id} aria-label="What's this?" aria-describedby={id}>
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M9.5 9.5a2.5 2.5 0 1 1 3.75 2.17c-.75.43-1.25.9-1.25 1.83v.5M12 17v.01" />
    </svg>
  </button>
  <!-- An empty title, so hovering the open description doesn't bring up the same text again. -->
  <span {id} class="help-tip" popover title="">{text}</span>
{/snippet}

<div class="app">
  <header>
    <img src={logo} alt="" width="36" height="36" />
    <div>
      <h1>D-Sep</h1>
      <p>{@render description()}</p>
    </div>
    <div class="menu" bind:this={menu}>
      <button
        bind:this={cog}
        class="icon-button"
        aria-label="Menu"
        title="Menu"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onclick={() => (menuOpen = !menuOpen)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M10.37 5.19L10.6 2.6h2.8l.23 2.59A7 7 0 0 1 15.66 6.03l1.99-1.67 1.99 1.99-1.67 1.99a7 7 0 0 1 .84 2.03l2.59.23v2.8l-2.59.23a7 7 0 0 1-.84 2.03l1.67 1.99-1.99 1.99-1.99-1.67a7 7 0 0 1-2.03.84l-.23 2.59h-2.8l-.23-2.59a7 7 0 0 1-2.03-.84l-1.99 1.67-1.99-1.99 1.67-1.99a7 7 0 0 1-.84-2.03L2.6 13.4v-2.8l2.59-.23a7 7 0 0 1 .84-2.03L4.36 6.35l1.99-1.99 1.99 1.67a7 7 0 0 1 2.03-.84z"
          />
          <circle cx="12" cy="12" r="3" />
        </svg>
      </button>
      {#if menuOpen}
        <div
          class="menu-list"
          role="menu"
          aria-label="Menu"
          tabindex="-1"
          onkeydown={menuKeys}
          {@attach (list) => list.querySelector("button")?.focus()}
        >
          <button role="menuitem" onclick={() => showSheet("about")}>About</button>
          <button role="menuitem" onclick={visitRepo}>
            GitHub
            <svg class="external" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M14 4h6v6M20 4l-9 9M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10" />
            </svg>
          </button>
          <button role="menuitem" onclick={() => showSheet("help")}>Help</button>
          <div class="menu-separator" role="separator"></div>
          <div class="menu-theme" role="group" aria-labelledby="theme-label">
            <span class="menu-label" id="theme-label">Theme</span>
            <!-- Picking one leaves the menu open, so the change can be seen and undone. -->
            <div class="segmented">
              {#each THEMES as option (option.theme)}
                <button
                  role="menuitemradio"
                  aria-checked={theme === option.theme}
                  class:on={theme === option.theme}
                  title={option.title}
                  onclick={() => (theme = option.theme)}
                >
                  {option.name}
                </button>
              {/each}
            </div>
          </div>
        </div>
      {/if}
    </div>
  </header>

  {#if !root && !scanning}
    <button class="dropzone" class:hover={dragging} onclick={browse} disabled={busy}>
      <svg class="drop-icon" viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M3 7.5A2.5 2.5 0 0 1 5.5 5h3.6a2 2 0 0 1 1.4.6L12 7h6.5A2.5 2.5 0 0 1 21 9.5v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.5z"
        />
      </svg>
      <strong>Drop a folder here</strong>
      <span>or click to choose one</span>
    </button>
    {#if here}
      {@const own = here}
      <div class="folder">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M3 7.5A2.5 2.5 0 0 1 5.5 5h3.6a2 2 0 0 1 1.4.6L12 7h6.5A2.5 2.5 0 0 1 21 9.5v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.5z"
          />
        </svg>
        <div class="folder-text" title={own}>
          <strong>{splitPath(own).name}</strong>
          <span>The folder D-Sep is in</span>
        </div>
        <button class="secondary small" onclick={() => scan(own)} disabled={busy}>Use this folder</button>
      </div>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  {:else if phase === "done" && summary}
    {@const result = summary}
    <!-- Every clip was already where it would go, and nothing went wrong: not a success to celebrate. -->
    {@const nothing = clean && placed === 0 && result.sidecars === 0}
    <section class="result">
      <div class="result-icon" class:warn={!clean} class:idle={nothing}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          {#if nothing}
            <path d="M7 12h10" />
          {:else if clean}
            <path d="M5 12.5l4.5 4.5L19 7.5" />
          {:else}
            <path d="M12 7v6.5M12 17v.01" />
          {/if}
        </svg>
      </div>
      <h2 bind:this={resultHeading} tabindex="-1">
        {result.cancelled ? "Stopped" : !clean ? "Done, with problems" : nothing ? "Nothing to do" : "Done"}
      </h2>
      <p class="muted">
        {#if placed}
          {resultLine(placed, sorted)} <b title={result.folder}>{splitPath(result.folder).name}</b>
        {:else if result.existing}
          {existingNote(result.existing)}
        {:else}
          No clips were {VERBS[verbFor(sorted.action, sorted.mode, sorted.destination)].done}.
        {/if}
      </p>

      <div class="stats totals">
        {#each LOG_KINDS as kind (kind)}
          {@const dir = result.folders[kind]}
          {#if dir}
            <button class="stat {kind} openable" title="Open {dir}" onclick={() => reveal(dir)}>
              <span class="stat-label">
                <i class="dot"></i>{FOLDERS[kind]}
                <svg class="external" viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M14 4h6v6M20 4l-9 9M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10" />
                </svg>
              </span>
              <strong>{result[kind]}</strong>
            </button>
          {:else}
            <div class="stat {kind}">
              <span class="stat-label"><i class="dot"></i>{FOLDERS[kind]}</span>
              <strong>{result[kind]}</strong>
            </div>
          {/if}
        {/each}
      </div>

      {#if result.sidecars || (result.existing && placed) || (sorted.verify && placed) || result.cancelled}
        <ul class="notes">
          {#if result.sidecars}
            <li>
              {plural(result.sidecars, "sidecar file")}
              {#if !placed}
                {result.sidecars === 1 ? "was" : "were"} added next to clips already in place.
              {:else}
                {sorted.action === "rename" ? "got the same prefix" : "went along with them"}.
              {/if}
            </li>
          {/if}
          {#if result.existing && placed}<li>{existingNote(result.existing)}</li>{/if}
          {#if sorted.verify && placed}<li>Every copy was read back and checked against its original.</li>{/if}
          {#if result.cancelled}<li>Stopped early. Unfinished clips were left untouched.</li>{/if}
        </ul>
      {/if}

      {#if result.failed.length}
        <div class="failures">
          <strong>Couldn't {VERBS[workVerb].name.toLowerCase()} {plural(result.failed.length, "file")}</strong>
          <ul>
            <!-- Not keyed: the list never changes once shown, and a duplicate key would throw. -->
            {#each result.failed as failure}
              <li><b>{failure.file}</b> — {failure.reason}</li>
            {/each}
          </ul>
        </div>
      {/if}
      {#if notice}<p class="notice" role="status">{notice}</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}

      <div class="actions">
        <button class="primary" onclick={() => reveal(result.folder)}>Open folder</button>
        {#if result.undoable}
          <button class="secondary" title="Move everything back where it was" onclick={undo}>Undo</button>
        {/if}
        <button
          class="secondary"
          title={result.undoable ? `Scan ${folder.name} again. Undo won't be offered after that.` : `Scan ${folder.name} again`}
          onclick={() => scan(root)}
        >
          Sort again
        </button>
        <button class="secondary" onclick={browse}>Choose another folder…</button>
      </div>
    </section>
  {:else}
    <div class="folder">
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M3 7.5A2.5 2.5 0 0 1 5.5 5h3.6a2 2 0 0 1 1.4.6L12 7h6.5A2.5 2.5 0 0 1 21 9.5v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.5z"
        />
      </svg>
      <div class="folder-text" title={scanning ? scanning.path : root}>
        <strong>{folder.name}</strong>
        <!-- Cut at the start: the folders nearest this one are the ones worth seeing. -->
        {#if folder.parent}<span class="cut-start"><bdi>{folder.parent}</bdi></span>{/if}
      </div>
      <button class="secondary small" onclick={browse} disabled={busy}>Change</button>
    </div>

    {#if notice}<p class="notice" role="status">{notice}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <div class="stats">
      {#each LOG_KINDS as kind (kind)}
        {@const ofKind = clipsOf[kind]}
        <div class="stat {kind}">
          <span class="stat-label"><i class="dot"></i>{FOLDERS[kind]}</span>
          <strong>{ofKind.length}</strong>
          <span class="stat-sub">{ofKind.length ? formatBytes(bytes(ofKind, false)) : "none"}</span>
        </div>
      {/each}
      <div class="stat">
        <span class="stat-label">Other</span>
        <strong>{otherCount}</strong>
        {#if unreadable}
          <span class="stat-sub danger" title="Hover a clip's Unreadable badge to see why">{unreadable} unreadable</span>
        {:else}
          <span class="stat-sub">left untouched</span>
        {/if}
      </div>
    </div>

    <div class="list" bind:this={list} tabindex="-1" aria-label="Clips">
      {#if scanning}
        <div class="list-empty" role="status">
          <span class="spinner"></span>
          <span>
            {#if scanning.probed}
              Reading clip {scanning.probed} of {scanning.found}…
            {:else if scanning.found}
              Found {plural(scanning.found, "video")}…
            {:else}
              Looking for videos…
            {/if}
          </span>
          <button class="secondary small" onclick={stopScan}>Cancel</button>
        </div>
      {:else if clips.length === 0}
        <div class="list-empty">
          <strong>No videos in this folder{recursive ? " or its subfolders" : ""}</strong>
          {#if !recursive}<button class="link" onclick={() => setRecursive(true)}>Look in subfolders too</button>{/if}
        </div>
      {:else}
        {#each clips as clip (clip.path)}
          {@const tag = badge(clip)}
          {@const prefix = newPrefix(clip)}
          {@const target = targetOf(clip)}
          {@const file = splitPath(clip.name).name}
          {@const dir = clip.name.slice(0, clip.name.length - file.length)}
          {@const already = placedAlready.has(clip.path)}
          <div class="row {rowState(clip)}" class:untouched={!isLog(clip.profile.kind)} class:in-place={already}>
            <span class="name" title={target ? `${clip.name} → ${target}` : clip.name}>
              {#if prefix}{dir}<span class="new-prefix {clip.profile.kind}">{prefix}</span>{file}{:else}{clip.name}{/if}
            </span>
            <span class="meta">
              {#if already}<span class="already">already sorted</span>{/if}
              {#if clip.sidecars.length}
                <span class="sidecars">+{clip.sidecars.map((s) => extension(s.path)).join(" ")}</span>
              {/if}
              {formatBytes(clip.size)}
            </span>
            <span class="badge {tag.tone}" title={tag.title}>{tag.text}</span>
          </div>
        {/each}
      {/if}
    </div>

    {#if working && progress}
      <div class="panel progress">
        <span class="sr-only" aria-live="polite">{announcement}</span>
        <div class="progress-head">
          <span>{doing} <b>{Math.max(progress.index, 1)}</b> of {progress.count}</span>
          <span class="muted">
            <!-- Verifying goes over every clip twice, so its bytes would add up to twice the clips' size. -->
            {#if sorted.verify}
              {Math.round(percent)}%
            {:else}
              {formatBytes(progress.bytesDone)} of {formatBytes(progress.bytesTotal)}
            {/if}
            {#if showRate}· {formatBytes(rate)}/s · about {formatDuration(timeLeft)} left{/if}
          </span>
        </div>
        <div
          class="bar"
          role="progressbar"
          aria-label="Progress"
          aria-valuenow={Math.round(percent)}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuetext={sorted.verify
            ? `${Math.round(percent)}%`
            : `${formatBytes(progress.bytesDone)} of ${formatBytes(progress.bytesTotal)}`}
        >
          <div style:width="{percent}%"></div>
        </div>
        <div class="progress-foot">
          <!-- Cut at the start, so the file's own name stays in view. -->
          <span class="progress-file cut-start"><bdi>{progress.file}</bdi></span>
          <button class="secondary small" onclick={stop} disabled={stopping}>{stopping ? "Stopping…" : "Cancel"}</button>
        </div>
      </div>
    {:else}
      <div class="panel controls">
        <span class="subtitle" id="separate-by">Separate by:</span>
        <div
          class="choices"
          role="radiogroup"
          aria-labelledby="separate-by"
          tabindex="-1"
          onkeydown={(e) =>
            radioKeys(
              e,
              CHOICES.map((c) => c.action),
              action,
              (value) => (action = value),
            )}
        >
          {#each CHOICES as choice (choice.action)}
            <button
              role="radio"
              aria-checked={action === choice.action}
              tabindex={action === choice.action ? 0 : -1}
              class="choice"
              class:on={action === choice.action}
              onclick={() => (action = choice.action)}
            >
              <span class="choice-head">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d={choice.icon} /></svg>
                {choice.title}
                <i class="radio"></i>
              </span>
              <span class="choice-example">
                {#if choice.action !== "rename"}D-Log\{/if}{#if choice.action !== "folders"}<b>{prefixes.dlog}</b>{/if}clip.mp4
              </span>
            </button>
          {/each}
        </div>

        {#if action !== "folders"}
          <div class="prefixes">
            <span>Prefix</span>
            {#each LOG_KINDS as kind (kind)}
              <label class="prefix {kind}">
                <i class="dot"></i>{FOLDERS[kind]}
                <!-- No maxlength: it counts UTF-16 units and could cut an emoji in half. cleanPrefix cuts instead. -->
                <input
                  value={prefixes[kind]}
                  placeholder={DEFAULT_PREFIXES[kind]}
                  spellcheck="false"
                  autocomplete="off"
                  aria-invalid={prefixCheck.bad.includes(kind)}
                  oninput={(e) => editPrefix(kind, e.currentTarget)}
                />
              </label>
            {/each}
          </div>
        {/if}

        <div class="where">
          <span class="where-item">
            <span class="where-label">Save to</span>
            <span class="where-path" title={destination ?? root}>
              {destination ? splitPath(destination).name : "Current folder"}
            </span>
            <button class="link" onclick={chooseDestination} disabled={busy}>
              {destination ? "Change…" : "Change destination…"}
            </button>
            {#if destination}<button class="link" onclick={() => (destination = null)}>Save here instead</button>{/if}
          </span>
          {#if nested}
            <!-- Names stay in each clip's own folder, so this is turned off for them rather than taken away, which
                 would move the controls around it. -->
            <span
              class="where-item"
              class:off={action === "rename"}
              title={action === "rename" ? "Names stay in each clip's own folder" : undefined}
            >
              <span class="where-label" id="layout-label">Subfolders</span>
              <div
                class="segmented"
                role="radiogroup"
                aria-labelledby="layout-label"
                aria-disabled={action === "rename"}
                tabindex="-1"
                onkeydown={(e) =>
                  radioKeys(
                    e,
                    LAYOUTS.map((l) => l.layout),
                    layout,
                    (value) => (layout = value),
                  )}
              >
                {#each LAYOUTS as option (option.layout)}
                  <button
                    role="radio"
                    aria-checked={layout === option.layout}
                    tabindex={layout === option.layout ? 0 : -1}
                    class:on={layout === option.layout}
                    title={option.title}
                    disabled={action === "rename"}
                    onclick={() => (layout = option.layout)}
                  >
                    {option.name}
                  </button>
                {/each}
              </div>
            </span>
          {/if}
        </div>

        <div class="options">
          <div
            class="segmented"
            role="radiogroup"
            aria-label="Copy or move"
            tabindex="-1"
            onkeydown={(e) =>
              radioKeys(
                e,
                MODES.map((m) => m.mode),
                mode,
                (value) => (mode = value),
              )}
          >
            {#each MODES as option (option.mode)}
              <button
                role="radio"
                aria-checked={mode === option.mode}
                tabindex={mode === option.mode ? 0 : -1}
                class:on={mode === option.mode}
                title={option.mode === "copy"
                  ? action === "rename" && !destination
                    ? "Keep the originals, and add renamed copies next to them"
                    : "Keep the originals and work on copies"
                  : action === "rename" && !destination
                    ? "Rename the originals"
                    : "Move the originals"}
                onclick={() => (mode = option.mode)}
              >
                <!-- Names in place: a move is a rename, and saying so shows that a copy duplicates every clip. -->
                {option.mode === "move" && action === "rename" && !destination ? "Rename" : option.name}
              </button>
            {/each}
          </div>
          <!-- The "?" sits outside the label, so clicking it doesn't toggle the checkbox. -->
          <span class="with-help" title={SUBFOLDERS_HELP}>
            <label class="check">
              <input
                type="checkbox"
                checked={recursive}
                disabled={busy}
                onchange={(e) => setRecursive(e.currentTarget.checked)}
              />
              Include subfolders
            </label>
            {@render help("subfolders-help", SUBFOLDERS_HELP)}
          </span>
          {#if sidecarKinds}
            <span class="with-help" title={SIDECARS_HELP}>
              <label class="check">
                <input type="checkbox" bind:checked={sidecars} disabled={busy} />
                Bring {sidecarKinds} files along
              </label>
              {@render help("sidecars-help", SIDECARS_HELP)}
            </span>
          {/if}
          <!-- Always here, so switching between copy and move doesn't move the controls; off when nothing is copied. -->
          <span class="with-help" title={copies ? VERIFY_HELP : NO_VERIFY_HELP}>
            <label class="check" class:off={!copies}>
              <input
                type="checkbox"
                checked={verify && copies}
                disabled={busy || !copies}
                onchange={(e) => (verify = e.currentTarget.checked)}
              />
              Verify copies
            </label>
            {@render help("verify-help", copies ? VERIFY_HELP : NO_VERIFY_HELP)}
          </span>
        </div>
        <!-- The first hint keeps room for two lines, so a shorter or longer one doesn't move the controls above. -->
        {#if prefixProblem}
          <p class="hint first problem" role="alert">{prefixProblem}</p>
        {:else if prefixNote && action !== "folders"}
          <p class="hint first problem" role="status">{prefixNote}</p>
        {:else}
          <p class="hint first">{plan}</p>
        {/if}
        {#if keeping && !prefixProblem}
          <!-- Also catches a prefix like "DJI_" that the camera's own names start with. -->
          <p class="hint">
            {keeping === 1
              ? "1 clip already starts with its prefix and keeps its name."
              : `${keeping} clips already start with their prefix and keep their names.`}
          </p>
        {/if}
        {#if placedAlready.size && toSort.length}
          <p class="hint">
            {placedAlready.size === 1 ? "1 clip is" : `${placedAlready.size} clips are`} already in place and will be skipped.
          </p>
        {/if}
        {#if short && space}
          <p class="hint problem">Only {formatBytes(space.free)} is free there, and this needs about {formatBytes(needed)}.</p>
        {/if}
        <button class="primary big" onclick={start} disabled={busy || toSort.length === 0 || !!prefixProblem}>
          {#if toSort.length}
            {VERBS[verb].name} {plural(toSort.length, "clip")}
            {#if verb !== "rename"}<span class="button-sub">· {formatBytes(sortBytes)}</span>{/if}
          {:else if logClips.length}
            {logClips.length === 1 ? "The clip is" : `All ${logClips.length} clips are`} already in place
          {:else}
            No D-Log, D-Log 2 or D-Log M clips here
          {/if}
        </button>
      </div>
    {/if}
  {/if}

  {#if dragging && (root || scanning)}
    <div class="overlay">Drop to scan this folder</div>
  {/if}
</div>

<!-- Escape closes it natively. The card fills the dialog, so a click on the dialog itself is on the backdrop. -->
<dialog
  bind:this={dialog}
  class="sheet"
  aria-labelledby="sheet-title"
  onclick={(e) => {
    if (e.target === e.currentTarget) dialog.close();
  }}
>
  <div class="sheet-body">
    <button class="icon-button close" aria-label="Close" title="Close" onclick={() => dialog.close()}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
    </button>
    {#if sheet === "about"}
      <div class="about">
        <img src={logo} alt="" width="72" height="72" />
        <h2 id="sheet-title">D-Sep</h2>
        {#if version}<span class="version">v{version}</span>{/if}
        <p class="muted">{@render description()}</p>
        <p class="credits">© 2026 ZTCS · MIT License</p>
        <div class="about-links">
          <button class="link" onclick={visitRepo}>GitHub</button>
          <button class="link" onclick={() => showSheet("licenses")}>Third-party licenses</button>
        </div>
      </div>
    {:else if sheet === "licenses"}
      <h2 id="sheet-title">Third-party licenses</h2>
      <pre class="licenses">{licenses || "Reading…"}</pre>
    {:else if sheet === "help"}
      <h2 id="sheet-title">How to use D-Sep</h2>
      <ol class="steps">
        <li>
          <b>Choose a folder</b>
          Drop a folder with DJI clips onto the window, or click the drop area to pick one. Nothing changes yet.
          Turn on <em>Include subfolders</em> to also look in the folders inside it.
        </li>
        <li>
          <b>Pick how to separate</b>
          <em>Folders</em> sorts clips into D-Log, D-Log 2 and D-Log M subfolders. <em>Names</em> puts a prefix like
          <span class="new-prefix dlog">{prefixes.dlog}</span> in front of their names. <em>Folders + Names</em> does both.
        </li>
        <li>
          <b>Copy or move</b>
          <em>Copy</em> keeps the originals where they are. <em>Move</em> moves them, or renames them in place.
          <em>Save to</em> another folder to take clips off a card straight into a project.
        </li>
        <li>
          <b>Start</b>
          Press the button at the bottom. You can cancel at any time; the clip in progress is left as it was.
        </li>
      </ol>
      <ul class="tips">
        <li>Clips in other profiles, like HLG or Rec.709, stay where they are.</li>
        <li>Nothing is ever overwritten. Clips that are already sorted are skipped.</li>
        <li>Files with the same name as a clip, like .LRF proxies and .SRT telemetry, go along with it, unless you turn that off.</li>
        <li>A move can be undone from the result screen, until you scan again.</li>
      </ul>
    {/if}
  </div>
</dialog>

<style>
  .app {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px 20px;
    /* A window too short for every control scrolls, rather than cutting off the button at the bottom. */
    overflow-y: auto;
  }

  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-bottom: 4px;
  }

  h1 {
    font-size: 17px;
    font-weight: 650;
    letter-spacing: -0.01em;
    line-height: 1.2;
  }

  header p,
  .muted {
    color: var(--muted);
  }

  header p {
    font-size: 12.5px;
    text-wrap: balance;
  }

  header p span,
  .about p span {
    white-space: nowrap;
  }

  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* Menu */

  .menu {
    position: relative;
    margin-left: auto;
  }

  .icon-button {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: 0;
    border-radius: 9px;
    background: none;
    color: var(--muted);
    transition:
      color 0.15s,
      background 0.15s;
  }

  .icon-button:hover,
  .icon-button[aria-expanded="true"] {
    background: var(--surface-2);
    color: var(--text);
  }

  .icon-button svg {
    width: 20px;
    height: 20px;
  }

  .menu-list {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    min-width: 168px;
    padding: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 11px;
    box-shadow:
      0 12px 32px -8px rgb(0 0 0 / 0.22),
      0 2px 6px -2px rgb(0 0 0 / 0.1);
    transform-origin: top right;
    animation: pop 0.12s ease-out;
  }

  .menu-list > button {
    display: flex;
    align-items: center;
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: 7px;
    background: none;
    text-align: left;
  }

  .menu-list > button:hover,
  .menu-list > button:focus-visible {
    outline: none;
    background: var(--surface-2);
  }

  .menu-separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }

  .menu-theme {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 6px 6px;
  }

  .menu-label {
    padding-left: 4px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
  }

  .menu-theme .segmented {
    display: flex;
  }

  .menu-theme .segmented button {
    flex: 1;
    padding: 3px 12px;
  }

  .menu-theme .segmented button:not(.on):hover {
    color: var(--text);
  }

  .external {
    width: 14px;
    height: 14px;
    margin-left: auto;
    color: var(--faint);
  }

  /* About and Help */

  .sheet {
    width: min(420px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    /* Centers it; the global reset takes away the browser's own auto margin. */
    margin: auto;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 24px 64px -16px rgb(0 0 0 / 0.35);
    scrollbar-width: thin;
    scrollbar-color: var(--border-strong) transparent;
  }

  .sheet[open] {
    animation: pop 0.15s ease-out;
  }

  .sheet::backdrop {
    background: color-mix(in srgb, var(--bg) 72%, transparent);
    backdrop-filter: blur(3px);
  }

  .sheet-body {
    position: relative;
    padding: 18px 22px 22px;
  }

  .close {
    position: absolute;
    top: 15px;
    right: 14px;
  }

  .sheet h2 {
    margin-bottom: 14px;
    padding-right: 40px;
  }

  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 14px 8px 6px;
    text-align: center;
  }

  .about img {
    margin-bottom: 6px;
  }

  .sheet .about h2 {
    margin: 0;
    padding: 0;
    font-size: 20px;
  }

  .about p {
    margin-top: 6px;
    text-wrap: balance;
  }

  .about .credits {
    color: var(--faint);
    font-size: 12px;
  }

  .about-links {
    display: flex;
    gap: 16px;
    margin-top: 4px;
    font-size: 12.5px;
  }

  .licenses {
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--surface-2);
    color: var(--muted);
    font: 11px/1.45 Consolas, "Cascadia Mono", monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .version {
    padding: 2px 10px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .steps {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 0;
    list-style: none;
    counter-reset: step;
    color: var(--muted);
    font-size: 12.5px;
  }

  .steps li {
    position: relative;
    padding-left: 34px;
    counter-increment: step;
  }

  .steps li::before {
    content: counter(step);
    position: absolute;
    top: 0;
    left: 0;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--dlog-bg);
    color: var(--dlog);
    font-size: 12px;
    font-weight: 650;
  }

  .steps b {
    display: block;
    color: var(--text);
    font-size: 13.5px;
    font-weight: 600;
  }

  .steps em {
    color: var(--text);
    font-style: normal;
    font-weight: 600;
  }

  .tips {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 16px;
    padding: 10px 12px 10px 28px;
    border-radius: 10px;
    background: var(--surface-2);
    color: var(--muted);
    font-size: 12.5px;
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.97);
    }
  }

  /* Empty state */

  .dropzone {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    background: var(--surface);
    border: 1.5px dashed var(--input-border);
    border-radius: 14px;
    color: var(--muted);
    transition:
      border-color 0.15s,
      background 0.15s;
  }

  .dropzone:hover:not(:disabled),
  .dropzone.hover {
    border-color: var(--faint);
    background: var(--surface-hover);
  }

  .dropzone.hover {
    border-style: solid;
    border-color: var(--dlog);
  }

  .dropzone strong {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
    margin-top: 8px;
  }

  .drop-icon {
    width: 44px;
    height: 44px;
    stroke-width: 1.3;
    color: var(--faint);
  }

  /* Folder bar */

  .folder {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 10px 10px 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
  }

  .folder svg {
    width: 20px;
    height: 20px;
    flex: none;
    color: var(--muted);
  }

  .folder-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .folder-text strong {
    font-weight: 600;
  }

  .folder-text span,
  .folder-text strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .folder-text span {
    color: var(--faint);
    font-size: 12px;
  }

  /* Overflows, and gets its ellipsis, at the start rather than the end. The <bdi> inside keeps the path in order. */
  .cut-start {
    direction: rtl;
    text-align: left;
  }

  /* Counters */

  .stats {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(0, 1fr);
    gap: 10px;
  }

  .stats.totals {
    width: 100%;
    max-width: 420px;
    margin: 6px 0 2px;
  }

  .stat {
    display: flex;
    flex-direction: column;
    padding: 10px 14px 11px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    text-align: left;
  }

  .stat-label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
  }

  .stat.dlog .stat-label {
    color: var(--dlog);
  }

  .stat.dlog2 .stat-label {
    color: var(--dlog2);
  }

  .stat.dlogm .stat-label {
    color: var(--dlogm);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
  }

  .stat strong {
    font-size: 26px;
    font-weight: 650;
    line-height: 1.25;
    font-variant-numeric: tabular-nums;
  }

  .stat-sub {
    color: var(--faint);
    font-size: 12px;
  }

  /* Short windows: leave the room to the clip list. */
  @media (max-height: 680px) {
    .stat {
      padding: 8px 12px 9px;
    }

    .stat strong {
      font-size: 22px;
    }

    .stat-sub {
      display: none;
    }
  }

  /* Clip list */

  .list {
    flex: 1;
    min-height: 80px;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    scrollbar-width: thin;
    scrollbar-color: var(--border-strong) transparent;
  }

  .list-empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--muted);
  }

  .list-empty strong {
    color: var(--text);
    font-weight: 600;
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 12px;
    padding: 7px 10px 7px 14px;
    border-bottom: 1px solid var(--border);
    /* Rows scrolled out of view skip layout and paint, so a folder of thousands of clips stays quick. */
    content-visibility: auto;
    contain-intrinsic-size: auto 35px;
  }

  .row:last-child {
    border-bottom: 0;
  }

  .row.active {
    background: var(--surface-hover);
  }

  .row.done .name,
  .row.untouched .name,
  .row.in-place .name {
    color: var(--muted);
  }

  .already {
    margin-right: 6px;
    font-size: 11px;
  }

  .row.done .badge::before {
    content: "✓ ";
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .new-prefix {
    font-weight: 650;
  }

  .new-prefix.dlog {
    color: var(--dlog);
  }

  .new-prefix.dlog2 {
    color: var(--dlog2);
  }

  .new-prefix.dlogm {
    color: var(--dlogm);
  }

  .meta {
    color: var(--faint);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .sidecars {
    margin-right: 6px;
    font-size: 11px;
    letter-spacing: 0.02em;
  }

  .badge {
    min-width: 62px;
    padding: 2px 9px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--muted);
    font-size: 11.5px;
    font-weight: 600;
    text-align: center;
    white-space: nowrap;
  }

  .badge.dlog {
    background: var(--dlog-bg);
    color: var(--dlog);
  }

  .badge.dlog2 {
    background: var(--dlog2-bg);
    color: var(--dlog2);
  }

  .badge.dlogm {
    background: var(--dlogm-bg);
    color: var(--dlogm);
  }

  .badge.error {
    color: var(--danger);
  }

  /* Bottom panel */

  /* Stays at the bottom when a short window scrolls, so the button that starts the work is always in view. */
  .panel {
    position: sticky;
    bottom: 0;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
  }

  /* Sits closer to the choices it names than the panel's gap would put it. */
  .subtitle {
    margin-bottom: -4px;
    color: var(--muted);
    font-size: 12.5px;
    font-weight: 600;
  }

  .choices {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .choice {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 8px 11px 9px;
    border: 1px solid var(--border-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--muted);
    text-align: left;
  }

  .choice:hover {
    background: var(--surface-hover);
  }

  .choice.on {
    border-color: var(--text);
    box-shadow: inset 0 0 0 1px var(--text);
    color: var(--text);
  }

  .choice-head {
    display: flex;
    align-items: center;
    gap: 5px;
    font-weight: 600;
  }

  .choice-head svg {
    width: 16px;
    height: 16px;
    flex: none;
  }

  .radio {
    flex: none;
    width: 14px;
    height: 14px;
    margin-left: auto;
    border: 1.5px solid var(--input-border);
    border-radius: 50%;
  }

  .choice.on .radio {
    border: 4.5px solid var(--text);
  }

  .choice-example {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--faint);
    font-size: 12px;
  }

  .choice-example b {
    color: var(--dlog);
    font-weight: 650;
  }

  .prefixes {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 16px;
    color: var(--muted);
  }

  .prefix {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    font-weight: 600;
  }

  .prefix.dlog {
    color: var(--dlog);
  }

  .prefix.dlog2 {
    color: var(--dlog2);
  }

  .prefix.dlogm {
    color: var(--dlogm);
  }

  .prefix input {
    width: 68px;
    height: 28px;
    margin-left: 2px;
    padding: 0 8px;
    border: 1px solid var(--input-border);
    border-radius: 7px;
    background: var(--bg);
    color: var(--text);
    font-weight: 600;
    user-select: text;
  }

  .prefix input[aria-invalid="true"] {
    border-color: var(--warn);
    box-shadow: 0 0 0 1px var(--warn);
  }

  .prefix input::placeholder {
    color: var(--faint);
    font-weight: 400;
  }

  .options {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 18px;
  }

  .segmented {
    display: inline-flex;
    gap: 2px;
    padding: 3px;
    background: var(--surface-2);
    border-radius: 9px;
  }

  .segmented button {
    padding: 4px 16px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--muted);
    font-weight: 600;
  }

  .segmented button.on {
    background: var(--segment-on);
    color: var(--text);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.14);
  }

  .check {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    color: var(--muted);
  }

  .check input {
    width: 15px;
    height: 15px;
    margin: 0;
    accent-color: var(--primary);
  }

  /* Each "?" anchors its own description. */
  .with-help {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    anchor-scope: --help;
  }

  .help {
    display: flex;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--faint);
    cursor: help;
    transition: color 0.15s;
    anchor-name: --help;
  }

  .help svg {
    width: 15px;
    height: 15px;
  }

  .with-help:hover .help {
    color: var(--muted);
  }

  /* Above its "?", centered on it, and moved in to stay clear of the window's edges. */
  .help-tip {
    position-anchor: --help;
    position-area: top;
    inset: auto;
    max-width: 260px;
    margin: 0 8px 6px;
    padding: 6px 9px;
    text-wrap: pretty;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow:
      0 12px 32px -8px rgb(0 0 0 / 0.22),
      0 2px 6px -2px rgb(0 0 0 / 0.1);
    font-size: 12.5px;
  }

  .hint {
    color: var(--muted);
    font-size: 12.5px;
  }

  .hint.problem {
    color: var(--warn);
  }

  /* Two lines' room. */
  .hint.first {
    min-height: 2.9em;
  }

  /* An option that doesn't apply to the others chosen. */
  .off {
    opacity: 0.5;
  }

  /* Buttons */

  .primary,
  .secondary {
    height: 36px;
    padding: 0 16px;
    border-radius: 10px;
    font-weight: 600;
    white-space: nowrap;
  }

  .primary {
    border: 0;
    background: var(--primary);
    color: var(--primary-text);
  }

  .primary:hover:not(:disabled) {
    opacity: 0.88;
  }

  .primary:disabled {
    opacity: 0.35;
  }

  .primary.big {
    position: relative;
    overflow: hidden;
    height: 42px;
    font-size: 14px;
    background: linear-gradient(110deg, var(--cta-from), var(--cta-via), var(--cta-to)) 0 0 / 200% 100%;
    color: #fff;
    text-shadow: 0 1px 1px rgba(0, 0, 0, 0.15);
    box-shadow: 0 2px 8px -2px var(--cta-glow);
    transition:
      background-position 0.4s ease,
      box-shadow 0.2s ease,
      transform 0.2s ease;
  }

  /* A soft sheen that sweeps across on hover. */
  .primary.big::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 30%, rgba(255, 255, 255, 0.28) 50%, transparent 70%);
    transform: translateX(-100%);
    pointer-events: none;
  }

  .primary.big:hover:not(:disabled) {
    opacity: 1;
    background-position: 100% 0;
    box-shadow: 0 8px 22px -6px var(--cta-glow);
    transform: translateY(-1px);
  }

  .primary.big:hover:not(:disabled)::after {
    transform: translateX(100%);
    transition: transform 0.7s ease;
  }

  .primary.big:active:not(:disabled) {
    transform: translateY(0);
    box-shadow: 0 2px 8px -2px var(--cta-glow);
  }

  .primary.big:disabled {
    opacity: 1;
    background: var(--surface-2);
    color: var(--faint);
    text-shadow: none;
    box-shadow: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .primary.big,
    .primary.big:hover:not(:disabled)::after {
      transition: none;
    }

    .primary.big:hover:not(:disabled) {
      transform: none;
    }

    .primary.big::after {
      display: none;
    }

    .menu-list,
    .sheet[open] {
      animation: none;
    }
  }

  .button-sub {
    opacity: 0.85;
    font-weight: 500;
  }

  .secondary {
    border: 1px solid var(--border-strong);
    background: var(--surface);
  }

  .secondary:hover:not(:disabled) {
    background: var(--surface-hover);
  }

  .secondary:disabled {
    opacity: 0.5;
  }

  .secondary.small {
    height: 30px;
    padding: 0 12px;
    font-size: 12.5px;
  }

  .link {
    border: 0;
    background: none;
    color: var(--dlog);
    font-weight: 600;
  }

  .link:hover {
    text-decoration: underline;
  }

  /* Progress */

  .progress-head,
  .progress-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .progress-head b {
    font-variant-numeric: tabular-nums;
  }

  .progress-head .muted {
    font-size: 12.5px;
    font-variant-numeric: tabular-nums;
  }

  .bar {
    height: 8px;
    overflow: hidden;
    background: var(--surface-2);
    border-radius: 999px;
  }

  .bar div {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, var(--dlog), var(--dlog2));
    transition: width 0.12s linear;
  }

  .progress-file {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--faint);
    font-size: 12.5px;
  }

  /* Result */

  .result {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 24px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    text-align: center;
  }

  .result-icon {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    margin-bottom: 6px;
    border-radius: 50%;
    background: var(--ok-bg);
    color: var(--ok);
  }

  .result-icon.warn {
    background: var(--warn-bg);
    color: var(--warn);
  }

  .result-icon.idle {
    background: var(--surface-2);
    color: var(--muted);
  }

  .result-icon svg {
    width: 28px;
    height: 28px;
    stroke-width: 2.4;
  }

  h2 {
    font-size: 18px;
    font-weight: 650;
  }

  .notes {
    padding: 0;
    list-style: none;
    color: var(--muted);
    font-size: 12.5px;
  }

  .failures {
    width: 100%;
    max-width: 420px;
    max-height: 140px;
    overflow-y: auto;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--surface-2);
    color: var(--muted);
    font-size: 12.5px;
    text-align: left;
    user-select: text;
  }

  .failures strong {
    color: var(--danger);
  }

  .failures ul {
    padding-left: 16px;
    margin-top: 4px;
  }

  .failures b {
    color: var(--text);
    font-weight: 600;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 10px;
    margin-top: 12px;
  }

  /* A result card that opens its folder. */
  .stat.openable {
    font: inherit;
    color: inherit;
    transition:
      background 0.15s,
      border-color 0.15s;
  }

  .stat.openable:hover {
    background: var(--surface-hover);
    border-color: var(--border-strong);
  }

  .stat .external {
    width: 12px;
    height: 12px;
    margin-left: auto;
    color: var(--faint);
  }

  .stat-sub.danger {
    color: var(--danger);
  }

  /* Save to, and where clips from subfolders go. */
  .where {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 18px;
    font-size: 12.5px;
  }

  .where-item {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .where-label {
    color: var(--muted);
    font-weight: 600;
  }

  .where-path {
    min-width: 0;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }

  .where .segmented button {
    padding: 2px 12px;
  }

  .link:disabled {
    opacity: 0.5;
    text-decoration: none;
    cursor: default;
  }

  .notice {
    padding: 9px 12px;
    border-radius: 10px;
    background: var(--surface-2);
    color: var(--muted);
    font-size: 12.5px;
  }

  /* Read out by screen readers, not shown. */
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  /* Misc */

  .error {
    padding: 9px 12px;
    border-radius: 10px;
    background: var(--surface-2);
    color: var(--danger);
    font-size: 12.5px;
    user-select: text;
  }

  .spinner {
    width: 22px;
    height: 22px;
    border: 2.5px solid var(--border-strong);
    border-top-color: var(--text);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Fixed, so it covers the window even when a short one has scrolled. */
  .overlay {
    position: fixed;
    z-index: 2;
    inset: 8px;
    display: grid;
    place-items: center;
    border: 2px solid var(--dlog);
    border-radius: 16px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(3px);
    font-size: 15px;
    font-weight: 600;
    pointer-events: none;
  }
</style>
