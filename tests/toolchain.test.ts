// Toolchain invariant: ci.yml and release.yml each pin the Rust toolchain (`dtolnay/rust-toolchain@<commit> #
// X.Y.Z`, the commit of the action's X.Y.Z branch), and Cargo.toml declares `rust-version`. A pin bumped in one
// workflow but not the other lets CI pass on a toolchain the release build doesn't use; a pin above `rust-version`
// means nothing proves the declared minimum still builds. The workflow comments say "bump them together"; this test
// enforces it, reading the version from the comment.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const read = (...p: string[]) => readFileSync(path.join(root, ...p), "utf8");

describe("Rust toolchain pins", () => {
  const pins = Object.fromEntries(
    ["ci.yml", "release.yml"].map((file) => [
      file,
      [...read(".github", "workflows", file).matchAll(/dtolnay\/rust-toolchain@[0-9a-f]{40} # ([\w.]+)/g)].map((m) => m[1]),
    ]),
  );
  const rustVersion = /^rust-version\s*=\s*"([^"]+)"/m.exec(read("src-tauri", "Cargo.toml"))?.[1];

  it("finds one pin per Rust job and Cargo.toml's rust-version", () => {
    expect(pins["ci.yml"]).toHaveLength(1);
    expect(pins["release.yml"]).toHaveLength(1);
    expect(rustVersion).toMatch(/^\d+\.\d+(\.\d+)?$/);
  });

  it("both workflows pin the same exact release, equal to rust-version", () => {
    const [major, minor, patch = "0"] = rustVersion!.split(".");
    const expected = `${major}.${minor}.${patch}`;
    expect(pins, `pin both workflows to the commit of dtolnay/rust-toolchain's ${expected} branch, or change rust-version too`).toEqual({
      "ci.yml": [expected],
      "release.yml": [expected],
    });
  });
});
