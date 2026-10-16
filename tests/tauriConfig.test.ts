// Tauri config invariants. Each of these is a one-line edit that nothing else would flag:
//
// - The window CSP keeps the webview off the network and limits scripts to the app's own files. D-Sep makes no
//   requests of its own (everything goes through IPC), so a remote origin or 'unsafe-eval' showing up here is a
//   mistake, not a feature.
// - The capabilities grant the window only the plugin permissions the UI uses. Anything broader (`dialog:default`
//   adds message/save dialogs, `fs:*` or `shell:*` would open the disk or processes to the webview) has to be a
//   deliberate change to this test too.
// - The identifier and names can't change once installs exist: WebView2 keeps its data, including the saved
//   options in localStorage, in a folder named after the identifier, and the NSIS installer finds the previous
//   install (folder, uninstall entry, shortcuts) by the product and binary names.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const readJson = <T>(...p: string[]) => JSON.parse(readFileSync(path.join(root, ...p), "utf8")) as T;

const conf = readJson<{
  identifier: string;
  productName: string;
  mainBinaryName: string;
  app: { security: { csp: string } };
}>("src-tauri", "tauri.conf.json");
const capability = readJson<{ windows: string[]; permissions: string[] }>("src-tauri", "capabilities", "default.json");

const directives = new Map(
  conf.app.security.csp
    .split(";")
    .map((d) => d.trim().split(/\s+/))
    .filter((parts) => parts[0])
    .map(([name, ...sources]) => [name, sources] as const),
);

describe("window CSP", () => {
  it("allows no remote origin and no eval", () => {
    const sources = [...directives.values()].flat();
    const remote = sources.filter(
      (s) => s.includes("*") || (/^https?:/.test(s) && s !== "http://ipc.localhost") || s === "'unsafe-eval'",
    );
    expect(remote).toEqual([]);
  });

  it("runs only the app's own scripts", () => {
    const scripts = directives.get("script-src") ?? directives.get("default-src");
    expect(scripts).toBeDefined();
    expect(scripts!.filter((s) => !["'self'", "ipc:", "http://ipc.localhost"].includes(s))).toEqual([]);
  });
});

describe("capabilities", () => {
  it("grant the main window only the permissions the UI uses", () => {
    expect(capability.windows).toEqual(["main"]);
    expect(capability.permissions).toEqual([
      "core:app:allow-version",
      "core:event:allow-listen",
      "core:event:allow-unlisten",
      "core:window:allow-set-theme",
      "core:window:allow-show",
      "dialog:allow-open",
    ]);
  });
});

describe("names existing installs depend on", () => {
  it("keeps the identifier, so saved options survive an update", () => {
    expect(conf.identifier).toBe("com.ztcs.dsep");
  });

  it("keeps the product and binary names, so the installer updates the existing install in place", () => {
    expect({ productName: conf.productName, mainBinaryName: conf.mainBinaryName }).toEqual({
      productName: "D-Sep",
      mainBinaryName: "D-Sep",
    });
  });
});
