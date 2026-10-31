// Writes public/third-party-notices.txt, which About shows: the licenses of the Rust crates built into D-Sep.exe, of
// Microsoft's WebView2 loader linked into it, and of the npm packages bundled into its UI, whose licenses ask for them
// to ship with the app. Run `npm run notices` after changing a dependency; tests/notices.test.ts fails while the file
// names versions the lockfiles don't have, or lists a package without a license text.

import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const LICENSE_FILE = /^(licen[cs]e|copying|notice|copyright)([-_.].*)?$/i;

// Crates that ship without a license file, and the folder in scripts/licenses/ with the texts from their repository.
// selectors' repository (servo/stylo) has none either, so it gets the Servo project's MPL-2.0 file from cssparser's.
const KEPT = {
  "alloc-stdlib": "rust-alloc-no-stdlib",
  selectors: "servo",
  "unic-char-property": "rust-unic",
  "unic-char-range": "rust-unic",
  "unic-common": "rust-unic",
  "unic-ucd-ident": "rust-unic",
  "unic-ucd-version": "rust-unic",
  "webview2-com": "webview2-rs",
  "webview2-com-sys": "webview2-rs",
};
const kept = (folder) => path.join(root, "scripts", "licenses", folder);

/** The texts of the license files in a package's folder. */
function licenseTexts(dir) {
  return readdirSync(dir)
    .filter((file) => LICENSE_FILE.test(file))
    .sort()
    .map((file) => readFileSync(path.join(dir, file), "utf8").replace(/\r\n/g, "\n").trim());
}

// Rust: the crates the Windows exe is built from. Only normal dependencies are followed, as build scripts and proc
// macros run while it's built and don't ship in it. cargo metadata doesn't keep apart the features a crate gets only
// for a build script, so a few crates only those use are listed too, which costs nothing but lines.
const metadata = JSON.parse(
  execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--locked", "--filter-platform", "x86_64-pc-windows-msvc"],
    { cwd: path.join(root, "src-tauri"), encoding: "utf8", maxBuffer: 1 << 28 },
  ),
);
const packages = new Map(metadata.packages.map((p) => [p.id, p]));
const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
const crates = new Map();
for (const queue = [metadata.resolve.root]; queue.length; ) {
  for (const dep of nodes.get(queue.pop()).deps) {
    const crate = packages.get(dep.pkg);
    const procMacro = crate.targets.some((t) => t.kind.includes("proc-macro"));
    if (crates.has(dep.pkg) || procMacro || !dep.dep_kinds.some((k) => k.kind === null)) continue;
    crates.set(dep.pkg, crate);
    queue.push(dep.pkg);
  }
}

// npm: the packages the UI bundle is built from, as Vite sees them.
const moduleIds = new Set();
await build({
  root,
  logLevel: "silent",
  build: { write: false },
  plugins: [
    {
      name: "list-bundled-modules",
      generateBundle(_, bundle) {
        for (const chunk of Object.values(bundle)) if (chunk.type === "chunk") chunk.moduleIds.forEach((id) => moduleIds.add(id));
      },
    },
  ],
});
const npmDirs = new Set();
for (const id of moduleIds) {
  // The last node_modules in the path: a package nested in another's is a package of its own.
  const at = [...id.matchAll(/node_modules[\\/](@[^\\/]+[\\/])?[^\\/]+/g)].at(-1);
  if (at) npmDirs.add(id.slice(0, at.index + at[0].length));
}

/** One line per package, and its license texts. */
const entries = [
  ...[...crates.values()].map((c) => ({
    section: "Rust crates",
    name: c.name,
    version: c.version,
    license: c.license ?? "see its license file",
    home: c.repository ?? c.homepage ?? "",
    texts: licenseTexts(KEPT[c.name] ? kept(KEPT[c.name]) : path.dirname(c.manifest_path)),
    authors: c.authors ?? [],
  })),
  // On MSVC, webview2-com-sys links WebView2LoaderStatic.lib from Microsoft's WebView2 SDK into the exe. Its license
  // is the one in the SDK's NuGet package, Microsoft.Web.WebView2.
  ...[...crates.values()]
    .filter((c) => c.name === "webview2-com-sys")
    .map((c) => ({
      section: "Other libraries",
      name: "Microsoft WebView2 loader",
      version: `(in webview2-com-sys ${c.version})`,
      license: "BSD-3-Clause",
      home: "https://www.nuget.org/packages/Microsoft.Web.WebView2",
      texts: licenseTexts(kept("webview2-sdk")),
      authors: [],
    })),
  ...[...npmDirs].map((dir) => {
    const pkg = JSON.parse(readFileSync(path.join(dir, "package.json"), "utf8"));
    const repository = typeof pkg.repository === "string" ? pkg.repository : (pkg.repository?.url ?? "");
    return {
      section: "npm packages",
      name: pkg.name,
      version: pkg.version,
      license: pkg.license ?? "see its license file",
      home: repository.replace(/^git\+/, "").replace(/\.git$/, ""),
      texts: licenseTexts(dir),
      authors: [],
    };
  }),
].sort((a, b) => a.section.localeCompare(b.section) || a.name.localeCompare(b.name) || a.version.localeCompare(b.version));

const rule = "-".repeat(78);
const out = [
  "Third-party software in D-Sep",
  "",
  "D-Sep is built with the software below, each part under its own license. Their license texts follow the",
  "list; packages with the very same text share one copy of it.",
];
for (const section of ["Rust crates", "Other libraries", "npm packages"]) {
  out.push("", section, "=".repeat(section.length));
  for (const e of entries.filter((e) => e.section === section)) {
    out.push(`${e.name} ${e.version}  ${e.license}${e.home ? `  ${e.home}` : ""}`);
  }
}

// Identical texts are written once, under every package that has them.
const byText = new Map();
for (const e of entries) {
  for (const text of e.texts) byText.set(text, [...(byText.get(text) ?? []), `${e.name} ${e.version}`]);
}
out.push("", "License texts", "=============");
for (const [text, users] of byText) out.push("", rule, users.join(", "), rule, "", text);

const bare = entries.filter((e) => !e.texts.length);
if (bare.length) {
  out.push("", rule, "Packages without a license file: each is under the license named, by its authors.", rule, "");
  for (const e of bare) out.push(`${e.name} ${e.version}  ${e.license}${e.authors.length ? `  by ${e.authors.join(", ")}` : ""}`);
}

writeFileSync(path.join(root, "public", "third-party-notices.txt"), `${out.join("\n")}\n`);
console.log(`${crates.size} crates and ${npmDirs.size} npm packages, ${byText.size} license texts.`);
