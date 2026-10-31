// Where a sort puts each log clip, worked out in the UI for the clip list's tooltips. It mirrors Job::new and
// output_dir in sort.rs by hand. No Svelte runes or Tauri imports, so this runs under plain Node in tests.

import type { Action, Clip, Layout, Prefixes } from "./api";
import { joinPath, relativeTo, splitPath } from "./format";

export type LogKind = "dlog" | "dlog2" | "dlogm";

export const LOG_KINDS: LogKind[] = ["dlog", "dlog2", "dlogm"];
export const FOLDERS: Record<LogKind, string> = { dlog: "D-Log", dlog2: "D-Log 2", dlogm: "D-Log M" };

/** The scanned folder, and the options that decide where clips go. */
export type Placing = { root: string; destination: string | null; action: Action; layout: Layout; prefixes: Prefixes };

export function isLog(kind: Clip["profile"]["kind"]): kind is LogKind {
  return (LOG_KINDS as string[]).includes(kind);
}

/** The prefix a log clip will get in front of its name, or "" when it keeps it. */
export function newPrefix(clip: Clip, { action, prefixes }: Pick<Placing, "action" | "prefixes">): string {
  const kind = clip.profile.kind;
  if (action === "folders" || !isLog(kind)) return "";
  const prefix = prefixes[kind];
  return splitPath(clip.name).name.toLowerCase().startsWith(prefix.toLowerCase()) ? "" : prefix;
}

/** Where a log clip will end up, relative to where the clips go when it's inside; "" for clips that stay. */
export function targetOf(clip: Clip, placing: Placing): string {
  const kind = clip.profile.kind;
  if (!isLog(kind)) return "";
  const path = joinPath(targetDir(clip, kind, placing), newPrefix(clip, placing) + splitPath(clip.name).name);
  const base = joinPath(placing.destination ?? placing.root, "");
  return path.toLowerCase().startsWith(base.toLowerCase()) ? path.slice(base.length) : path;
}

/** The folder a log clip goes to. Mirrors Job::new in sort.rs. */
function targetDir(clip: Clip, kind: LogKind, { root, destination, action, layout }: Placing): string {
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
