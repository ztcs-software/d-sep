// Release-gate invariant: the version string lives in five places, and the release workflow
// (.github/workflows/release.yml `prepare` job) HARD-FAILS when the first four disagree — but only after the
// PR has already merged to main. This test moves that failure to `npm test` time, before anything ships.
//
// Cargo.lock isn't compared by the workflow, so a forgotten `cargo update -p d-sep` would slip past it and
// fail later at CI's `cargo test --locked` instead, with an error that doesn't say "you bumped the version".
// package-lock.json carries the version twice (top level and `packages[""]`); `npm version` keeps both in
// step, a hand edit of package.json doesn't.
//
// Cargo.toml takes the first `version = "…"` line of the [package] section (dependency tables use inline
// `{ version = "…" }`, which never starts a line). Cargo.lock is matched on the `[[package]]` entry named
// `d-sep`: every dependency has the same shape, so the name is what keeps this from reading another crate's.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const read = (...p: string[]) => readFileSync(path.join(root, ...p), "utf8");

describe("release version agreement", () => {
  const pkg = (JSON.parse(read("package.json")) as { version?: unknown }).version;
  const lock = JSON.parse(read("package-lock.json")) as {
    version?: unknown;
    packages?: Record<string, { version?: unknown }>;
  };
  const tauri = (JSON.parse(read("src-tauri", "tauri.conf.json")) as { version?: unknown }).version;
  const cargo = /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(read("src-tauri", "Cargo.toml"))?.[1];
  // Line-walk rather than a multi-line regex: `version` is always the line right after `name`.
  const cargoLock = (() => {
    const lines = read("src-tauri", "Cargo.lock")
      .split("\n")
      .map((l) => l.trimEnd());
    const i = lines.indexOf('name = "d-sep"');
    return i === -1 ? undefined : /^version = "([^"]+)"$/.exec(lines[i + 1] ?? "")?.[1];
  })();

  const versions = {
    "package.json": pkg,
    "package-lock.json": lock.version,
    'package-lock.json packages[""]': lock.packages?.[""]?.version,
    "src-tauri/tauri.conf.json": tauri,
    "src-tauri/Cargo.toml": cargo,
    "src-tauri/Cargo.lock": cargoLock,
  };

  it("finds a plausible version in every file", () => {
    // A shape guard, so a broken parse fails with a clear message instead of passing vacuously.
    for (const [file, version] of Object.entries(versions)) {
      expect(version, `${file} version`).toMatch(/^\d+\.\d+\.\d+/);
    }
  });

  it("every file agrees with package.json", () => {
    const expected = Object.fromEntries(Object.keys(versions).map((file) => [file, pkg]));
    expect(
      versions,
      "bump with `npm version X.Y.Z --no-git-tag-version`, edit tauri.conf.json and Cargo.toml, " +
        "then refresh the lockfile with `cargo update -p d-sep`",
    ).toEqual(expected);
  });
});
