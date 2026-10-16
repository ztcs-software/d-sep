import { Channel, invoke } from "@tauri-apps/api/core";

export type Profile =
  | { kind: "dlog" }
  | { kind: "dlog2" }
  | { kind: "dlogm" }
  | { kind: "other"; label: string }
  | { kind: "unknown" };

export interface Sidecar {
  path: string;
  size: number;
}

export interface Clip {
  path: string;
  /** Path relative to the scanned folder. */
  name: string;
  size: number;
  profile: Profile;
  sidecars: Sidecar[];
  error: string | null;
}

export interface Scan {
  root: string;
  clips: Clip[];
}

export interface ScanProgress {
  /** Videos found so far. */
  found: number;
  /** How many of them have been read; 0 while folders are still being listed. */
  probed: number;
}

export type Mode = "copy" | "move";

/** Into the D-Log / D-Log 2 / D-Log M folders, a prefix on the name in place, or both. */
export type Action = "folders" | "rename" | "both";

/** For clips in subfolders: one set of profile folders at the top, or a set inside each subfolder. */
export type Layout = "gather" | "each";

export interface Prefixes {
  dlog: string;
  dlog2: string;
  dlogm: string;
}

export interface SortOptions {
  mode: Mode;
  action: Action;
  sidecars: boolean;
  /** Ignored for "folders". */
  prefixes: Prefixes;
  /** Where the clips go, subfolders kept; null for the scanned folder itself. */
  destination: string | null;
  /** Ignored for "rename". */
  layout: Layout;
  /** Read every copy back from the disk and compare it with its original. */
  verify: boolean;
}

export interface Progress {
  index: number;
  count: number;
  file: string;
  bytesDone: number;
  bytesTotal: number;
  /** Reading a copy back to compare it with its original, rather than copying. */
  verifying: boolean;
}

export interface Summary {
  dlog: number;
  dlog2: number;
  dlogm: number;
  sidecars: number;
  existing: number;
  failed: Failure[];
  cancelled: boolean;
  /** Where the clips went: the destination, or the scanned folder. */
  folder: string;
  /** For each profile, the folder its clips are in now, when that's a single folder. */
  folders: ProfileFolders;
  /** Whether files were moved, so undoSort has something to put back. */
  undoable: boolean;
}

export interface ProfileFolders {
  dlog: string | null;
  dlog2: string | null;
  dlogm: string | null;
}

export interface Failure {
  file: string;
  reason: string;
}

export interface Undone {
  /** Files moved back. */
  restored: number;
  /** Files still to move back, so undo can be tried again. */
  remaining: number;
  failed: Failure[];
  cancelled: boolean;
}

export interface DiskSpace {
  /** Bytes free on the drive the clips would go to. */
  free: number;
  /** Whether that's the drive the clips are on, where a move is just a rename. */
  sameDrive: boolean;
}

/** Scans a folder; null when cancelScan(id) stopped it. */
export function scanFolder(path: string, recursive: boolean, id: number, onProgress: (progress: ScanProgress) => void) {
  const channel = new Channel<ScanProgress>();
  channel.onmessage = onProgress;
  return invoke<Scan | null>("scan_folder", { path, recursive, id, onProgress: channel });
}

export const cancelScan = (id: number) => invoke<void>("cancel_scan", { id });

/** The clips of the last scan that sorting with `options` would leave alone, because they're already in place. */
export const inPlace = (options: SortOptions) => invoke<string[]>("in_place", { options });

/**
 * Sorts the clips from the last scan into "D-Log", "D-Log 2" and "D-Log M" folders, renames them, or both. `id`
 * numbers it for cancelSort.
 */
export function sortClips(options: SortOptions, id: number, onProgress: (progress: Progress) => void) {
  const channel = new Channel<Progress>();
  channel.onmessage = onProgress;
  return invoke<Summary>("sort_clips", { options, id, onProgress: channel });
}

/** Stops the sort or undo numbered `id`. */
export const cancelSort = (id: number) => invoke<void>("cancel_sort", { id });

/** Moves the files of the last move back. `id` numbers it for cancelSort. */
export function undoSort(id: number, onProgress: (progress: Progress) => void) {
  const channel = new Channel<Progress>();
  channel.onmessage = onProgress;
  return invoke<Undone>("undo_sort", { id, onProgress: channel });
}

/** Free space where the clips would go: the destination, or the scanned folder. */
export const diskSpace = (destination: string | null) => invoke<DiskSpace>("disk_space", { destination });

/** The folder D-Sep.exe is in; null for an installed D-Sep. */
export const exeFolder = () => invoke<string | null>("exe_folder");

/** Opens the scanned folder, or one the last sort put clips in. */
export const openFolder = (path: string) => invoke<void>("open_folder", { path });

/** Opens the project's GitHub page in the default browser. */
export const openRepo = () => invoke<void>("open_repo");
