// Third-party notices invariant: public/third-party-notices.txt, which About shows, lists the packages D-Sep is
// built from with the versions in the lockfiles. It's generated (`npm run notices`) and committed, so nothing
// rebuilds it when a dependency changes; this test fails until it's regenerated.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const read = (...p: string[]) => readFileSync(path.join(root, ...p), "utf8");

const notices = read("public", "third-party-notices.txt").replace(/\r\n/g, "\n");
/** The "name version" lines under a heading of the notices. */
function listed(section: string): string[] {
  const body = notices.split(`\n${section}\n${"=".repeat(section.length)}\n`)[1]?.split("\n\n")[0] ?? "";
  return body.split("\n").map((line) => line.split("  ")[0]);
}

const cargoLock = new Set(
  [...read("src-tauri", "Cargo.lock").matchAll(/^name = "([^"]+)"\r?\nversion = "([^"]+)"/gm)].map((m) => `${m[1]} ${m[2]}`),
);
const npmLock = JSON.parse(read("package-lock.json")) as { packages: Record<string, { version?: string }> };
const npm = new Set(
  Object.entries(npmLock.packages)
    .filter(([key]) => key.startsWith("node_modules/"))
    .map(([key, p]) => `${key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length)} ${p.version}`),
);

describe("third-party notices", () => {
  const crates = listed("Rust crates");
  const packages = listed("npm packages");

  it("lists the packages D-Sep is built from", () => {
    for (const name of ["tauri", "tauri-plugin-dialog", "rfd", "serde", "windows-sys"]) {
      expect(crates.some((c) => c.startsWith(`${name} `)), name).toBe(true);
    }
    for (const name of ["svelte", "@tauri-apps/api", "@tauri-apps/plugin-dialog"]) {
      expect(packages.some((p) => p.startsWith(`${name} `)), name).toBe(true);
    }
  });

  it("names the WebView2 loader that webview2-com-sys links into the exe", () => {
    expect(listed("Other libraries").some((l) => l.startsWith("Microsoft WebView2 loader "))).toBe(true);
  });

  it("has a license text for every package: add a crate's to scripts/licenses/ and KEPT if it ships none", () => {
    expect(notices).not.toContain("Packages without a license file");
  });

  it("names only versions the lockfiles have: run `npm run notices` after a dependency changes", () => {
    expect(crates.filter((c) => !cargoLock.has(c))).toEqual([]);
    expect(packages.filter((p) => !npm.has(p))).toEqual([]);
  });
});
