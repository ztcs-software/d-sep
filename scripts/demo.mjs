// Records the animation at the top of the README, docs/demo.gif: D-Sep's own UI in its light theme in Microsoft Edge,
// driven by Playwright, with the Rust side replaced by a fake that finds a made-up folder of DJI clips.
// Run `npm run demo` after a change the animation shows. It needs Windows, for Edge and the Segoe fonts the app and its
// title bar use, and ffmpeg on the PATH.

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const readJson = (...p) => JSON.parse(readFileSync(path.join(root, ...p), "utf8"));
const { width: WIDTH, height: HEIGHT } = readJson("src-tauri", "tauri.conf.json").app.windows[0];
const { version } = readJson("package.json");
/** The Windows 11 title bar drawn above the page. */
const TITLE_BAR = 32;
const FPS = 20;
const SORT_SECONDS = 3.5;

// The folder dropped on the window: log clips in all three profiles, mixed with clips in others that stay where they
// are, each with the .LRF proxy and .SRT telemetry a DJI drone records next to it.
const FOLDER = "D:\\Footage\\Dolomites";
const FOOTAGE = [
  // When it was shot, its profile, and its size in GB.
  ["071503", "dlogm", 1.42],
  ["071948", "dlogm", 2.07],
  ["072610", "HLG", 0.86],
  ["080122", "dlog", 1.18],
  ["080533", "dlog", 0.64],
  ["091207", "dlog2", 1.73],
  ["091844", "dlogm", 0.95],
  ["093016", "Rec.709", 0.41],
  ["101422", "dlog2", 1.26],
  ["102905", "dlog", 2.31],
  ["104411", "dlogm", 0.58],
];
const clips = FOOTAGE.map(([time, profile, gb], i) => {
  const name = `DJI_20260512${time}_${String(i + 1).padStart(4, "0")}_D.MP4`;
  const stem = `${FOLDER}\\${name.slice(0, -".MP4".length)}`;
  const size = Math.round(gb * 1024 ** 3);
  return {
    path: `${FOLDER}\\${name}`,
    name,
    size,
    profile: profile.startsWith("dlog") ? { kind: profile } : { kind: "other", label: profile },
    sidecars: [
      { path: `${stem}.LRF`, size: Math.round(size / 24) },
      { path: `${stem}.SRT`, size: 48_000 + i * 1_337 },
    ],
    error: null,
  };
});

/**
 * Stands in for Tauri in the page, before the UI loads: answers the commands it sends the way the Rust side does, with
 * progress at about the pace of a scan and of a copy on an SSD, and gives the recording `demoDrag` to send the
 * webview's drag-and-drop events.
 */
function fakeTauri({ folder, clips, version, sortSeconds }) {
  const FOLDERS = { dlog: "D-Log", dlog2: "D-Log 2", dlogm: "D-Log M" };
  const callbacks = new Map();
  const listeners = new Map();
  let nextId = 1;
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  /** Sends messages through a Channel, numbered as it expects them. */
  const sender = (channel) => {
    let index = 0;
    return (message) => callbacks.get(channel.id)?.({ index: index++, message });
  };

  async function scanFolder({ onProgress }) {
    const send = sender(onProgress);
    for (let found = 1; found <= clips.length; found++) {
      await sleep(20);
      send({ found, probed: 0 });
    }
    for (let probed = 1; probed <= clips.length; probed++) {
      await sleep(45);
      send({ found: clips.length, probed });
    }
    return { root: folder, clips };
  }

  async function sortClips({ options, onProgress }) {
    const send = sender(onProgress);
    const jobs = clips.filter((clip) => clip.profile.kind in FOLDERS);
    const bytesOf = (clip) => clip.size + (options.sidecars ? clip.sidecars.reduce((sum, s) => sum + s.size, 0) : 0);
    const base = options.destination ?? folder;
    const progress = { index: 0, count: jobs.length, file: "", bytesDone: 0, bytesTotal: 0, verifying: false };
    progress.bytesTotal = jobs.reduce((sum, clip) => sum + bytesOf(clip), 0);
    const perMs = progress.bytesTotal / (sortSeconds * 1000);
    const summary = {
      ...{ dlog: 0, dlog2: 0, dlogm: 0, sidecars: 0, existing: 0, failed: [], cancelled: false, folder: base },
      folders: { dlog: null, dlog2: null, dlogm: null },
      undoable: options.mode === "move",
    };
    for (const clip of jobs) {
      progress.index++;
      progress.file = clip.name;
      send({ ...progress });
      const start = progress.bytesDone;
      const end = start + bytesOf(clip);
      const began = performance.now();
      while (progress.bytesDone < end) {
        await sleep(33);
        progress.bytesDone = Math.min(end, Math.round(start + (performance.now() - began) * perMs));
        send({ ...progress });
      }
      const kind = clip.profile.kind;
      summary[kind]++;
      summary.folders[kind] = options.action === "rename" ? base : `${base}\\${FOLDERS[kind]}`;
      if (options.sidecars) summary.sidecars += clip.sidecars.length;
    }
    return summary;
  }

  const commands = {
    scan_folder: scanFolder,
    sort_clips: sortClips,
    in_place: () => [],
    disk_space: () => ({ free: 1.6 * 1024 ** 4, sameDrive: true }),
    exe_folder: () => null,
    cancel_scan: () => null,
    cancel_sort: () => null,
    open_folder: () => null,
    open_repo: () => null,
    "plugin:app|version": () => version,
    "plugin:window|set_theme": () => null,
    "plugin:window|show": () => null,
    "plugin:event|listen": ({ event, handler }) => {
      listeners.set(event, [...(listeners.get(event) ?? []), handler]);
      return handler;
    },
    "plugin:event|unlisten": () => null,
  };

  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    async invoke(cmd, args) {
      if (!(cmd in commands)) throw new Error(`The demo has no fake for ${cmd}`);
      return commands[cmd](args);
    },
    transformCallback(callback, once = false) {
      const id = nextId++;
      callbacks.set(id, (data) => {
        if (once) callbacks.delete(id);
        return callback?.(data);
      });
      return id;
    },
    unregisterCallback: (id) => callbacks.delete(id),
    runCallback: (id, data) => callbacks.get(id)?.(data),
    callbacks,
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event, id) => callbacks.delete(id) };

  /** Sends the webview a drag-and-drop event: "enter", "drop" or "leave". */
  window.demoDrag = (type, paths) => {
    const event = `tauri://drag-${type}`;
    const payload = { paths, position: { x: 0, y: 0 } };
    for (const handler of listeners.get(event) ?? []) callbacks.get(handler)?.({ event, id: 0, payload });
  };
}

/**
 * Draws what the page lacks to look like the app on Windows 11: the window's title bar and border, and a mouse pointer
 * that can carry a folder and show its clicks. The recording moves it through `window.demo`.
 */
function stage({ titleBar, folderName }) {
  const style = document.createElement("style");
  style.textContent = `
    html { padding-top: ${titleBar}px; }
    .demo-titlebar {
      position: fixed; z-index: 2147483645; inset: 0 0 auto; height: ${titleBar}px;
      display: flex; align-items: center; gap: 10px; padding-left: 12px;
      background: #ffffff; color: #1b1b1b; font: 12px "Segoe UI Variable Text", "Segoe UI", sans-serif;
    }
    .demo-titlebar img { width: 16px; height: 16px; }
    .demo-titlebar i { display: grid; place-items: center; width: 46px; height: 100%; font: normal 10px "Segoe Fluent Icons"; }
    .demo-titlebar i:first-of-type { margin-left: auto; }
    .demo-frame { position: fixed; z-index: 2147483646; inset: 0; border: 1px solid rgb(0 0 0 / 0.22); pointer-events: none; }
    .demo-pointer { position: fixed; z-index: 2147483647; left: 0; top: 0; pointer-events: none; }
    .demo-pointer svg { display: block; }
    .demo-ghost {
      position: absolute; left: 10px; top: 20px; display: none; flex-direction: column; align-items: center; gap: 2px;
      opacity: 0.88; font: 12px "Segoe UI Variable Text", "Segoe UI", sans-serif; color: #1b1b1b;
    }
    .demo-ghost span { padding: 0 6px; border-radius: 4px; background: rgb(0 120 212 / 0.22); }
    .demo-carrying .demo-ghost { display: flex; }
    .demo-click {
      position: fixed; z-index: 2147483646; width: 30px; height: 30px; margin: -15px 0 0 -15px; border-radius: 50%;
      background: rgb(128 128 128 / 0.4); pointer-events: none; animation: demo-click 0.4s ease-out forwards;
    }
    @keyframes demo-click { from { scale: 0.3; opacity: 1; } to { scale: 1.3; opacity: 0; } }
  `;
  document.head.append(style);
  document.body.insertAdjacentHTML(
    "beforeend",
    `<div class="demo-titlebar">
      <img src="/src/assets/logo.svg" alt="" /><span>D-Sep</span><i>\uE921</i><i>\uE922</i><i>\uE8BB</i>
    </div>
    <div class="demo-frame"></div>
    <div class="demo-pointer">
      <svg width="14" height="22" viewBox="0 0 14 22">
        <path d="M1.5 1.5V17l3.8-3.6 2.6 6.1 2.4-1-2.5-6h5z" fill="#fff" stroke="#000" stroke-linejoin="round" />
      </svg>
      <div class="demo-ghost">
        <svg width="44" height="36" viewBox="0 0 44 36">
          <path d="M3 6a3 3 0 0 1 3-3h10l4 4h18a3 3 0 0 1 3 3v20a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" fill="#e0a526" />
          <rect x="3" y="11" width="38" height="22" rx="3" fill="#ffc83d" />
        </svg>
        <span>${folderName}</span>
      </div>
    </div>`,
  );
  const pointer = document.querySelector(".demo-pointer");
  window.demo = {
    point: (x, y) => (pointer.style.translate = `${x}px ${y}px`),
    carry: (on) => pointer.classList.toggle("demo-carrying", on),
    click(x, y) {
      const ring = document.createElement("div");
      ring.className = "demo-click";
      ring.style.left = `${x}px`;
      ring.style.top = `${y}px`;
      ring.onanimationend = () => ring.remove();
      document.body.append(ring);
    },
  };
  return document.querySelector(".demo-titlebar img").decode();
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Plays the story and returns its frames, PNGs with the second each was drawn at. */
async function record(browser, url) {
  const viewport = { width: WIDTH, height: HEIGHT + TITLE_BAR };
  const context = await browser.newContext({ viewport, colorScheme: "light" });
  const page = await context.newPage();
  page.on("pageerror", (error) => console.error(error));
  page.on("console", (message) => {
    // The app has no favicon, and WebView2 doesn't ask for one.
    if (message.type() === "error" && !message.location().url.endsWith("/favicon.ico")) console.error(message.text());
  });
  await page.addInitScript(fakeTauri, { folder: FOLDER, clips, version, sortSeconds: SORT_SECONDS });
  await page.goto(url);
  await page.locator(".dropzone").waitFor();
  await page.evaluate(stage, { titleBar: TITLE_BAR, folderName: FOLDER.split("\\").at(-1) });

  let at = { x: WIDTH * 0.66, y: HEIGHT + TITLE_BAR + 30 };
  /**
   * Glides the pointer to (x, y), easing in and out as a hand on a mouse does. The mouse goes with it, so what's under
   * it shows its hover, but not while dragging: Windows doesn't tell the webview where a drag is.
   */
  async function glide([x, y], ms, dragging = false) {
    const from = at;
    const start = Date.now();
    for (let t = 0; t < 1; await sleep(10)) {
      t = Math.min(1, (Date.now() - start) / ms);
      const eased = t < 0.5 ? 4 * t ** 3 : 1 - (2 - 2 * t) ** 3 / 2;
      at = { x: from.x + (x - from.x) * eased, y: from.y + (y - from.y) * eased };
      if (!dragging) await page.mouse.move(at.x, at.y);
      await page.evaluate(({ x, y }) => window.demo.point(x, y), at);
    }
  }
  async function click() {
    await page.evaluate(({ x, y }) => window.demo.click(x, y), at);
    await page.mouse.down();
    await sleep(70);
    await page.mouse.up();
  }
  /** A point on an element, `dx` and `dy` from its middle, where a hand would aim. */
  async function on(locator, dx = 0, dy = 0) {
    const box = await locator.boundingBox();
    return [box.x + box.width / 2 + dx, box.y + box.height / 2 + dy];
  }

  const cdp = await context.newCDPSession(page);
  const frames = [];
  cdp.on("Page.screencastFrame", ({ data, metadata, sessionId }) => {
    frames.push({ data, at: metadata.timestamp ?? Date.now() / 1000, arrived: Date.now() });
    cdp.send("Page.screencastFrameAck", { sessionId }).catch(() => {});
  });
  await page.evaluate(({ x, y }) => window.demo.point(x, y), at);
  await cdp.send("Page.startScreencast", { format: "png", maxWidth: WIDTH, maxHeight: HEIGHT + TITLE_BAR });
  await sleep(400);

  // Drag the folder in from below the window, and drop it.
  await page.evaluate(() => window.demo.carry(true));
  await glide([WIDTH * 0.6, HEIGHT + TITLE_BAR - 8], 300, true);
  await page.evaluate((folder) => window.demoDrag("enter", [folder]), FOLDER);
  await glide(await on(page.locator(".dropzone"), 6, 24), 600, true);
  await sleep(200);
  await page.evaluate((folder) => {
    window.demo.carry(false);
    window.demoDrag("drop", [folder]);
  }, FOLDER);
  await page.locator(".row").first().waitFor();
  await sleep(1000);

  // Folders + Names: the list shows the prefixes the clips will get.
  await glide(await on(page.getByRole("radio", { name: /^Folders \+ Names/ }), 10, -6), 500);
  await click();
  await sleep(1300);

  // Copy, then the result.
  await glide(await on(page.locator("button.primary.big"), 30, 2), 500);
  await sleep(150);
  await click();
  await sleep(150);
  // Out of the way, between the file name and Cancel.
  await glide(await on(page.locator(".progress-foot"), 80, -6), 500);
  await page.locator(".result").waitFor({ timeout: (SORT_SECONDS + 10) * 1000 });
  await sleep(500);
  await glide(await on(page.locator(".stat.dlogm.openable"), 12, 4), 500);
  await sleep(1300);

  await cdp.send("Page.stopScreencast");
  // Frames come only when something changes, so the last one is held until the story's end.
  const last = frames.at(-1);
  frames.push({ ...last, at: last.at + (Date.now() - last.arrived) / 1000 });
  await context.close();
  return frames;
}

/** Turns frames into a looping GIF, with a palette of its own and only the changed part of each frame stored. */
function encode(frames, out) {
  const dir = mkdtempSync(path.join(tmpdir(), "d-sep-demo-"));
  try {
    const list = ["ffconcat version 1.0"];
    for (const [i, frame] of frames.entries()) {
      const file = `${String(i).padStart(5, "0")}.png`;
      writeFileSync(path.join(dir, file), Buffer.from(frame.data, "base64"));
      const next = frames[i + 1];
      list.push(`file ${file}`, ...(next ? [`duration ${Math.max(0.001, next.at - frame.at).toFixed(4)}`] : []));
    }
    writeFileSync(path.join(dir, "frames.txt"), list.join("\n"));
    const filter =
      `fps=${FPS},split[a][b];[a]palettegen=stats_mode=full[p];` +
      "[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle";
    const args = ["-v", "error", "-y", "-f", "concat", "-i", path.join(dir, "frames.txt"), "-vf", filter, "-loop", "0", out];
    if (spawnSync("ffmpeg", args, { stdio: "inherit" }).status) throw new Error("ffmpeg couldn't make the GIF");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (spawnSync("ffmpeg", ["-version"], { stdio: "ignore" }).error) throw new Error("ffmpeg isn't on the PATH");
const server = await createServer({ root, logLevel: "error", server: { port: 1430, strictPort: false } });
await server.listen();
const browser = await chromium.launch({ channel: "msedge" });
try {
  const out = path.join(root, "docs", "demo.gif");
  encode(await record(browser, server.resolvedUrls.local[0]), out);
  console.log(`${path.relative(root, out)}: ${(statSync(out).size / 1024 ** 2).toFixed(1)} MB`);
} finally {
  await browser.close();
  await server.close();
}
