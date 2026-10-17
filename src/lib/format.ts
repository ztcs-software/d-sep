const UNITS = ["KB", "MB", "GB", "TB"];

/** Sizes the way Explorer shows them (1 KB = 1024 bytes). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${UNITS[unit]}`;
}

export function plural(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

/** Splits a folder path into its own name and the parent path. */
export function splitPath(path: string): { name: string; parent: string } {
  const trimmed = path.replace(/[\\/]+$/, "");
  const at = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  if (at < 0) return { name: trimmed || path, parent: "" };
  return { name: trimmed.slice(at + 1), parent: trimmed.slice(0, at) };
}

/** Upper-case extension, e.g. "LRF". */
export function extension(path: string): string {
  const name = path.slice(Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/")) + 1);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toUpperCase() : "";
}

/** A rough time left: "45 s", "3 min", "1 h 20 min". */
export function formatDuration(seconds: number): string {
  const s = Math.round(seconds);
  if (s < 60) return `${Math.max(1, s)} s`;
  const minutes = Math.round(s / 60);
  if (minutes < 60) return `${minutes} min`;
  const [h, m] = [Math.floor(minutes / 60), minutes % 60];
  return m ? `${h} h ${m} min` : `${h} h`;
}

/** `dir` and `name` joined with a backslash, as Windows writes paths. */
export function joinPath(dir: string, name: string): string {
  return `${dir.replace(/[\\/]+$/, "")}\\${name}`;
}

/** Whether two paths name the same folder, ignoring case and trailing separators as Windows does. */
export function sameFolder(a: string, b: string): boolean {
  const plain = (path: string) => path.replace(/[\\/]+$/, "").toLowerCase();
  return plain(a) === plain(b);
}

/** The part of `path` under `folder`, ignoring case as Windows does: "" for the folder itself, null for elsewhere. */
export function relativeTo(path: string, folder: string): string | null {
  const [inner, outer] = [path, folder].map((p) => p.replace(/[\\/]+$/, ""));
  if (inner.toLowerCase() === outer.toLowerCase()) return "";
  const under = inner.toLowerCase().startsWith(outer.toLowerCase()) && /[\\/]/.test(inner[outer.length]);
  return under ? inner.slice(outer.length + 1) : null;
}

/**
 * What to say once a drop has been scanned into `root`: a dropped file shows all of its folder, and of several
 * folders only the first is scanned. "" for a single folder.
 */
export function dropNotice(paths: string[], root: string): string {
  const inRoot = (path: string) => sameFolder(path, root) || sameFolder(splitPath(path).parent, root);
  if (!paths.every(inRoot)) return `One folder at a time: this is the first of the ${paths.length} you dropped.`;
  if (paths.length === 1 && sameFolder(paths[0], root)) return "";
  const files = paths.length === 1 ? "file is" : "files are";
  return `Showing everything in ${splitPath(root).name}, where the dropped ${files}.`;
}
