// IPC agreement: src/lib/api.ts mirrors the Rust side of the IPC boundary by hand: the command names, their
// arguments and every serialized type. There's no codegen, and neither `cargo check` nor `svelte-check` can see
// the other language, so drift only shows at runtime: "Command x not found", a missing argument, or a field
// that is silently `undefined` in the UI. This test reads both sides as text and moves those failures to
// `npm test` time.
//
// Tauri passes command arguments by name and expects camelCase keys for snake_case Rust parameters. Structs
// follow their `#[serde(rename_all)]`. Parameters Tauri injects itself (State, AppHandle, windows) aren't sent
// by the UI and are left out.

import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const rustDir = path.join(root, "src-tauri", "src");
const mainRs = readFileSync(path.join(rustDir, "main.rs"), "utf8");
const rust = readdirSync(rustDir)
  .filter((f) => f.endsWith(".rs"))
  .map((f) => readFileSync(path.join(rustDir, f), "utf8"))
  .join("\n");
const apiTs = readFileSync(path.join(root, "src", "lib", "api.ts"), "utf8");

/** Rust struct or enum ↔ the api.ts type that mirrors it. */
const MIRRORS = {
  structs: {
    Scan: "Scan",
    Clip: "Clip",
    Sidecar: "Sidecar",
    Options: "SortOptions",
    Prefixes: "Prefixes",
    Progress: "Progress",
    Summary: "Summary",
    Failure: "Failure",
    ScanProgress: "ScanProgress",
    ProfileFolders: "ProfileFolders",
    Undone: "Undone",
    DiskSpace: "DiskSpace",
  },
  enums: { Profile: "Profile", Mode: "Mode", Action: "Action", Layout: "Layout" },
} as const;

const INJECTED = /^(?:tauri::)?(?:State|AppHandle|Window|WebviewWindow|Webview)\b/;

const camel = (snake: string) => snake.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());
const sorted = (names: Iterable<string>) => [...names].sort();

const stripComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");

/** Splits on commas outside <>, () and []. */
function splitTopLevel(list: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = "";
  for (const ch of list) {
    if ("<([".includes(ch)) depth++;
    if (">)]".includes(ch)) depth--;
    if (ch === "," && depth === 0) {
      parts.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  parts.push(current);
  return parts.map((p) => p.trim()).filter(Boolean);
}

/** The body of the one `{ … }` block that starts at `pattern`. */
function block(source: string, pattern: RegExp, what: string): { body: string; before: string } {
  const matches = [...source.matchAll(new RegExp(pattern.source, "g"))];
  expect(matches.length, `exactly one ${what}`).toBe(1);
  const start = matches[0].index! + matches[0][0].length;
  let depth = 1;
  let i = start;
  while (depth > 0 && i < source.length) {
    if (source[i] === "{") depth++;
    if (source[i] === "}") depth--;
    i++;
  }
  return { body: source.slice(start, i - 1), before: source.slice(0, matches[0].index) };
}

/** The `#[serde(…)]` arguments right above a Rust item, or "". */
function serdeAttribute(before: string): string {
  const attributes = /((?:\s*#\[[^\]]*\])*)\s*(?:pub(?:\([^)]*\))?\s+)?$/.exec(before)?.[1] ?? "";
  return /#\[serde\(([^)]*)\)\]/.exec(attributes)?.[1] ?? "";
}

function rename(name: string, attribute: string): string {
  const rule = /rename_all\s*=\s*"([^"]+)"/.exec(attribute)?.[1];
  if (rule === "camelCase") return camel(name);
  if (rule === "lowercase") return name.toLowerCase();
  expect(rule, `unsupported rename_all on ${name}`).toBeUndefined();
  return name;
}

/** The JSON field names of a Rust struct. */
function rustFields(name: string): string[] {
  const { body, before } = block(rust, new RegExp(`\\bstruct ${name}\\s*\\{`), `Rust struct ${name}`);
  const code = stripComments(body);
  expect(code, `field-level serde attributes on ${name} aren't understood by this test`).not.toMatch(/#\[serde/);
  const attribute = serdeAttribute(before);
  return sorted(
    [...code.matchAll(/^\s*(?:pub(?:\([^)]*\))?\s+)?(\w+)\s*:/gm)].map((m) => rename(m[1], attribute)),
  );
}

/** The top-level member names of an api.ts interface. */
function tsFields(name: string): string[] {
  let { body } = block(apiTs, new RegExp(`export interface ${name}\\s*\\{`), `api.ts interface ${name}`);
  body = stripComments(body);
  // Drop nested object and parameter lists, so only this interface's own members are left.
  for (let before = ""; before !== body; ) {
    before = body;
    body = body.replace(/\{[^{}]*\}/g, "{}").replace(/\([^()]*\)/g, "()");
  }
  return sorted([...body.matchAll(/(\w+)\??\s*:/g)].map((m) => m[1]));
}

/** A Rust enum's serde attribute and its variants, as the JSON names them, with whether each carries data. */
function rustVariants(name: string): { attribute: string; variants: Map<string, boolean> } {
  const { body, before } = block(rust, new RegExp(`\\benum ${name}\\s*\\{`), `Rust enum ${name}`);
  const code = stripComments(body);
  expect(code, `variant-level serde attributes on ${name} aren't understood by this test`).not.toMatch(/#\[serde/);
  const attribute = serdeAttribute(before);
  const variants = new Map(
    [...code.matchAll(/^\s*(\w+)\s*(\()?/gm)].map((m) => [rename(m[1], attribute), !!m[2]]),
  );
  return { attribute, variants };
}

/** The right-hand side of an api.ts type alias; it ends at the first `;` that ends a line. */
function tsAlias(name: string): string {
  const alias = new RegExp(`export type ${name}\\s*=([\\s\\S]*?);\\s*$`, "m").exec(apiTs)?.[1];
  expect(alias, `api.ts type ${name}`).toBeDefined();
  return stripComments(alias!);
}

describe("commands (main.rs ↔ api.ts)", () => {
  const registered = sorted(
    splitTopLevel(/generate_handler!\[([^\]]*)\]/.exec(mainRs)?.[1] ?? "").map((name) => name.split("::").pop()!),
  );
  const calls = [...apiTs.matchAll(/\binvoke(?:<[^>]*>)?\(\s*"(\w+)"\s*(?:,\s*\{([^}]*)\})?\s*\)/g)];

  it("parses the handler list and every invoke() call", () => {
    // Parse guards, so a refactor fails loudly instead of the checks below passing on empty lists.
    expect(registered).toContain("scan_folder");
    expect(calls.length).toBe([...apiTs.matchAll(/\binvoke(?:<[^>]*>)?\(/g)].length);
  });

  it("every registered command has an api.ts wrapper, and every wrapper a registered command", () => {
    expect(sorted(new Set(calls.map((c) => c[1])))).toEqual(registered);
  });

  for (const command of registered) {
    it(`${command} gets the arguments it takes`, () => {
      const signature = new RegExp(`#\\[tauri::command\\]\\s*(?:pub\\s+)?(?:async\\s+)?fn\\s+${command}\\s*\\(([^)]*)\\)`);
      const params = signature.exec(mainRs)?.[1];
      expect(params, `#[tauri::command] fn ${command} in main.rs`).toBeDefined();
      const expected = splitTopLevel(params!)
        .map((p) => /^(?:mut\s+)?(\w+)\s*:\s*(.+)$/s.exec(p)!)
        .filter(([, , type]) => !INJECTED.test(type.trim()))
        .map(([, name]) => camel(name));
      for (const call of calls.filter((c) => c[1] === command)) {
        const sent = splitTopLevel(call[2] ?? "").map((p) => p.split(":")[0].trim());
        expect(sorted(sent), `invoke("${command}") in api.ts`).toEqual(sorted(expected));
      }
    });
  }
});

describe("serialized types (Rust ↔ api.ts)", () => {
  it("every Serialize/Deserialize type in src-tauri/src is mirrored here", () => {
    const serde = [
      ...rust.matchAll(
        /#\[derive\([^)]*\b(?:Serialize|Deserialize)\b[^)]*\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum)\s+(\w+)/g,
      ),
    ].map((m) => m[1]);
    expect(sorted(serde), "add the new type to api.ts and to MIRRORS in this test").toEqual(
      sorted([...Object.keys(MIRRORS.structs), ...Object.keys(MIRRORS.enums)]),
    );
  });

  for (const [rustName, tsName] of Object.entries(MIRRORS.structs)) {
    it(`${rustName} ↔ ${tsName}`, () => {
      expect(tsFields(tsName)).toEqual(rustFields(rustName));
    });
  }

  it("Profile ↔ the kinds and labels of the Profile union", () => {
    const { attribute, variants } = rustVariants("Profile");
    expect(attribute).toMatch(/tag\s*=\s*"kind"/);
    expect(attribute).toMatch(/content\s*=\s*"label"/);
    // Each union member is `{ kind: "…" }`, plus `label` when the variant carries data.
    const members = [...tsAlias("Profile").matchAll(/\{([^}]*)\}/g)].map((m) => m[1]);
    const kinds = new Map(members.map((m) => [/kind:\s*"(\w+)"/.exec(m)?.[1] ?? m, /\blabel\??\s*:/.test(m)]));
    expect(kinds).toEqual(variants);
  });

  for (const name of ["Mode", "Action", "Layout"] as const) {
    it(`${name} ↔ its string union`, () => {
      const { variants } = rustVariants(name);
      expect([...variants.values()].some(Boolean), `${name} variants carry no data`).toBe(false);
      const literals = [...tsAlias(name).matchAll(/"(\w+)"/g)].map((m) => m[1]);
      expect(sorted(literals)).toEqual(sorted(variants.keys()));
    });
  }
});
