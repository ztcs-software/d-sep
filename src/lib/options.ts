// The options D-Sep remembers between runs, kept in localStorage. No Svelte runes or Tauri imports, so this runs
// under plain Node in tests.

import type { Action, Layout, Mode, Prefixes } from "./api";

/** "auto" follows Windows. */
export type Theme = "light" | "dark" | "auto";

export type Options = {
  mode: Mode;
  action: Action;
  recursive: boolean;
  sidecars: boolean;
  prefixes: Prefixes;
  theme: Theme;
  layout: Layout;
  verify: boolean;
};

export const OPTIONS_KEY = "d-sep.options";
export const DEFAULT_PREFIXES: Prefixes = { dlog: "DL_", dlog2: "DL2_", dlogm: "DLM_" };
/** The same as MAX_PREFIX_CHARS in sort.rs. */
export const MAX_PREFIX = 16;

/**
 * The saved options. Each field falls back to its default on its own when it's missing or invalid, and all of
 * them do when nothing is saved, the JSON is broken, or storage can't be read.
 */
export function loadOptions(storage?: Pick<Storage, "getItem">): Options {
  let stored: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse((storage ?? localStorage).getItem(OPTIONS_KEY) ?? "{}");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) stored = parsed as Record<string, unknown>;
  } catch {
    // Unreadable: the defaults below.
  }
  const prefixes = (stored.prefixes ?? {}) as Partial<Record<keyof Prefixes, unknown>>;
  return {
    mode: stored.mode === "move" ? "move" : "copy",
    action: stored.action === "rename" || stored.action === "both" ? stored.action : "folders",
    recursive: stored.recursive !== false,
    sidecars: stored.sidecars !== false,
    prefixes: {
      dlog: cleanPrefix(prefixes.dlog) || DEFAULT_PREFIXES.dlog,
      dlog2: cleanPrefix(prefixes.dlog2) || DEFAULT_PREFIXES.dlog2,
      dlogm: cleanPrefix(prefixes.dlogm) || DEFAULT_PREFIXES.dlogm,
    },
    theme: stored.theme === "light" || stored.theme === "dark" ? stored.theme : "auto",
    layout: stored.layout === "each" ? "each" : "gather",
    verify: stored.verify === true,
  };
}

/** Saves the options; without storage, they just aren't remembered. */
export function saveOptions(options: Options, storage?: Pick<Storage, "setItem">): void {
  try {
    (storage ?? localStorage).setItem(OPTIONS_KEY, JSON.stringify(options));
  } catch {
    // Storage unavailable.
  }
}

/**
 * Drops control characters and what Windows forbids in a name, leading dots, "$" and spaces, which would
 * hide it from a scan, and anything past MAX_PREFIX characters. Counted by code point, as Rust counts chars, so an
 * emoji is one and is never cut in half.
 */
export function cleanPrefix(value: unknown): string {
  if (typeof value !== "string") return "";
  const cleaned = value.replace(/[<>:"/\\|?*\u0000-\u001f\u007f-\u009f]/g, "").replace(/^[\s.$]+/, "");
  return Array.from(cleaned).slice(0, MAX_PREFIX).join("");
}

/**
 * Whether a prefix starts with a name Windows keeps for a device, like "NUL.": "NUL.DJI_0001.MP4" couldn't be
 * opened. The same rule as is_device_name in sort.rs, which refuses it.
 */
export function isDevicePrefix(prefix: string): boolean {
  const dot = prefix.indexOf(".");
  return dot >= 0 && /^(con|prn|aux|nul|com\d|lpt\d)$/i.test(prefix.slice(0, dot).trimEnd());
}
