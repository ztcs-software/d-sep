# D-Sep — Feature Documentation

A small Windows desktop app that separates DJI clips shot in D-Log, D-Log 2
and D-Log M from everything else. D-Sep runs as a Tauri 2 app on Windows 10
and 11, works only on the folders you give it, and makes no network requests
of its own.

This document is the source of truth for the public documentation website. Each
section describes a user-visible feature, what problem it solves, and the
options that surround it. Internal implementation notes are kept out unless
they leak through the UX.

---

## 1. Profile detection

### 1.1 Why it's needed

DJI cameras tag every clip as Rec.709 in the standard color atom (`colr`),
even when it was shot in log. Players, editors and Explorer therefore can't
tell a D-Log clip from a normal one, and a card full of mixed footage has to
be sorted by hand before grading. D-Sep reads the profile the camera really
used and sorts on that.

### 1.2 What it reads

The real profile is stored in QuickTime metadata inside the file's `moov`
box, under the key `com.dji.camera.ColorGammaSxS`:

```
moov/meta/keys  →  com.dji.camera.ColorGammaSxS
moov/meta/ilst  →  "D-Log" | "D-Log2" | "D-Log M" | …
```

`moov/udta/meta` is checked as well, in case a firmware puts it there.
(Credit to [dji-clip-color](https://github.com/erik-sutton95/dji-clip-color)
for documenting the key; About says so too.)

- **Spelling doesn't matter** — only letters and digits are compared, ignoring
  case, so "D-Log M", "D-LogM" and "DLogM" are all D-Log M, and "D-Log2" is
  D-Log 2.
- **Other profiles** (HLG, Rec.709, …) are shown with the camera's own label,
  cut to 32 characters, and left untouched.
- **No profile** — a video with no DJI color metadata (not a DJI clip, or a
  camera that doesn't write it) is shown as *No profile* and left untouched.
- **Unreadable** — a file that can't be read is marked *Unreadable*, with the
  reason on hover, and left untouched (§3).

### 1.3 Fast on any file size

D-Sep doesn't read the video. It walks the MP4 box headers with a few seeks
and reads only the `keys` box and the one item it needs (each capped at
64 KB), so a probe takes a handful of tiny reads per clip whether the clip is
200 MB or 20 GB. Thumbnails stored next to the profile are never read. It
handles 64-bit box sizes and both QuickTime- and ISO-style `meta` boxes.

If a file's top-level box chain is broken (a mangled `mdat` size, say), D-Sep
searches the last 4 MB of the file for the metadata instead of giving up.

---

## 2. Choosing a folder

### 2.1 Start screen

- **Drop a folder here** — drop a folder anywhere on the window, or click the
  drop area to pick one. Nothing changes on disk until you press the button at
  the bottom (§4.9).
- **Use this folder** — the portable `.exe` (§12) also offers the folder it's
  in, so a copy kept on a memory card can sort that card with one click. The
  installed app doesn't, as its install folder holds no clips.

Once a folder is showing, its name and the path above it sit at the top (the
path is cut from the start, so the nearest folders stay in view), with a
**Change** button that opens the picker there. Dropping onto the window at
any time scans the dropped folder instead; a **Drop to scan this folder**
overlay says so. Drops are ignored while a scan, sort or undo is running.

- **A dropped file** scans its folder, and says *Showing everything in
  &lt;folder&gt;, where the dropped file is.*
- **Several folders** — only the first is scanned, and D-Sep says so: *One
  folder at a time: this is the first of the 3 you dropped.*

### 2.2 What a scan finds

- **Videos** — `.mp4` and `.mov` files, in any letter case.
- **Include subfolders** (on by default) — also looks in the folders inside
  the chosen one, at any depth. Toggling it rescans straight away. A subfolder
  that can't be read is skipped rather than failing the scan.
- **Skipped** — names starting with `.` or `$` (hidden files, macOS `._`
  resource forks, `$RECYCLE.BIN`), `System Volume Information`, and the
  `D-Log`, `D-Log 2` and `D-Log M` folders at the top of the scanned folder,
  so sorted clips aren't listed again. (Those that **Per folder** made inside
  subfolders are listed, and show as already sorted while the options stay
  the same.) Names that aren't valid Unicode are skipped too.
- **Cloud files** — a video that OneDrive (or another cloud drive) keeps
  online only isn't read, since reading it would download all of it. It shows
  as *Unreadable*: *It's only in the cloud. Make it available on this device,
  then scan again.*

Up to 8 clips are read at once, so one slow file holds up only itself.

### 2.3 Sidecars

DJI cameras write files next to each clip with the same name: `.LRF` proxies,
`.SRT` telemetry, and so on. A sidecar is any non-video file in the same
folder whose name, without its extension, matches the clip's, ignoring case.
Sidecars travel with their clip and get the same prefix (§4.7).

### 2.4 Scan progress and Cancel

While a scan runs, the clip list shows what it's doing — *Looking for
videos…*, *Found 120 videos…*, then *Reading clip 37 of 120…* — with a
**Cancel** button. A cancelled or failed scan brings back what was on screen
before; a cancelled rescan from **Include subfolders** also puts the checkbox
back.

---

## 3. The clip list

Above the list, one card per profile — **D-Log**, **D-Log 2**, **D-Log M** —
gives the number of clips and their total size, and an **Other** card counts
everything that will be left untouched, or how many files were unreadable
(hover a clip's *Unreadable* badge to see why).

Each row shows:

- **The clip's path** under the scanned folder. Clips are listed folder by
  folder, each folder's own clips before its subfolders'.
- **The new prefix** in front of the name, in the profile's color, when the
  clip will be renamed (§4.2).
- **Sidecars** as their extensions (`+LRF SRT`), and the clip's size.
- **A badge** — the profile (D-Log, D-Log 2, D-Log M), the camera's label for
  another profile, *No profile*, or *Unreadable*.
- **Where it goes** — hovering a log clip's badge or name shows the path it
  will end up at.

Clips that will be left untouched are dimmed. Clips that are **already in
place** — a sort with the current options would find them where it would put
them, sidecars included — are marked *already sorted* and aren't counted on
the button (§4.9). The marks are worked out again a moment after any option
that decides where clips go changes.

An empty folder says *No videos in this folder* and, when subfolders are off,
offers **Look in subfolders too**. Long lists stay quick: rows off screen
aren't drawn.

---

## 4. Sort options

The controls sit at the bottom of the window and stay in view in a short
window that scrolls.

### 4.1 Separate by

| Separate by | `DJI_0001.MP4` (D-Log) becomes |
| --- | --- |
| **Folders** | `D-Log\DJI_0001.MP4` |
| **Names** | `DL_DJI_0001.MP4`, in the same folder |
| **Folders + Names** | `D-Log\DL_DJI_0001.MP4` |

D-Log 2 clips go to `D-Log 2` and get `DL2_`; D-Log M clips go to `D-Log M`
and get `DLM_`. Each choice shows a small example with the current D-Log
prefix.

### 4.2 Prefixes

Shown for **Names** and **Folders + Names**: one field per profile, `DL_`,
`DL2_` and `DLM_` by default. A prefix:

- can be up to **16 characters**;
- can't contain `\ / : * ? " < > |` or control characters, or start with a
  dot, `$` or a space (the scan would then skip the renamed clips). Such
  characters are dropped as you type, and a note says why;
- can't be empty, or start with a device name and a dot, like `NUL.` or
  `COM1.` — Windows and most programs couldn't open the clip;
- with **Names**, must differ from the other two (ignoring case), or clips of
  two profiles would look the same. With **Folders + Names** they're in
  different folders, so the same prefix is fine.

A problem marks its field and disables the button until it's fixed.

**Never renames twice** — a clip whose name already starts with its prefix
(ignoring case) keeps its name. A hint counts them: *3 clips already start
with their prefix and keep their names.* This also catches a prefix like
`DJI_`, which every camera name starts with.

### 4.3 Copy, Move, Rename

- **Copy** (default) — keeps the originals and works on copies.
- **Move** — moves the originals. Within one drive a move is an instant
  rename, whatever the clip's size. To another drive it's a copy, flushed to
  the disk, and only then is the original deleted (§5.2).
- With **Names** and no destination, **Move** reads **Rename**: the clips are
  renamed where they are, and **Copy** adds renamed copies next to the
  originals.

A move can be undone (§7).

### 4.4 Save to

By default clips are sorted in the scanned folder (**Current folder**).
**Change destination…** picks another folder — to take clips off a card
straight into a project, for example — and **Save here instead** goes back.
With **Names** or **Per folder** (§4.5), clips keep their subfolders under the
destination. Picking the scanned folder
itself is the same as **Current folder**. The destination isn't remembered:
each session starts with the folder itself.

If the destination is deleted before the sort starts, D-Sep stops with *The
destination folder no longer exists.* and touches nothing.

### 4.5 Subfolders: Together or Per folder

Shown when some log clips are in subfolders, for **Folders** and **Folders +
Names**:

- **Together** (default) — one set of `D-Log` / `D-Log 2` / `D-Log M` folders
  for the folder and all its subfolders. Clips from subfolders join them.
- **Per folder** — a set inside each subfolder, next to its clips.

With **Names** it stays visible but turned off, since names stay in each
clip's own folder.

The profile folders are named `D-Log`, `D-Log 2` and `D-Log M`. Sorting one of
them sorts into it and its siblings, never `D-Log\D-Log`: a D-Log M clip found
in `D-Log` goes to the `D-Log M` folder beside it.

### 4.6 Bring sidecars along

**Bring LRF, SRT files along** (on by default) appears when the log clips
have sidecars, and names the kinds found. Off, sidecars stay where they are.

### 4.7 What happens to sidecars

Sidecars are copied or moved with their clip and get the same prefix, and the
same ` (2)` suffix if the clip needs one (§5.1). Once a clip is placed, its
sidecars always follow, even if you press Cancel at that moment. A clip
that's already in place still gets any sidecars it's missing (after an
earlier sort with sidecars off, say) — copied even in a move, since the clip
itself stays where it is and keeps its own.

### 4.8 Verify copies

Off by default. On, each copy is read back from the disk itself — not from
Windows' cache — and compared byte for byte with its original before it gets
its real name. Worth it before wiping a card. It takes about twice as long. A
copy that doesn't match is deleted and reported (*The copy doesn't match the
original.*); its original is untouched, even in a move.

The checkbox is always shown, so switching between Copy and Move doesn't move
the controls around, but it's turned off when nothing is copied: a move
within one drive.

### 4.9 The plan and the button

A sentence under the options says what pressing the button will do, for
example:

- *Clips will be copied into D-Log and D-Log M folders in this folder. Clips
  from subfolders go there too.*
- *Clips will be moved and renamed into D-Log folders next to them, in the
  same subfolders of Project.*
- *Clips will be renamed where they are.*

It names only the profiles there are clips of. Below it, as they apply:

- *2 clips are already in place and will be skipped.*
- **Free space** — *Only 12 GB is free there, and this needs about 40 GB.*
  Checked whenever the view is ready or the destination changes, against the
  clips that will really be copied (a move within a drive needs no space).

The button names the work and counts only the clips not already in place,
with their size (and their sidecars', when those come along) except for a
rename: **Copy 12 clips · 38 GB**, **Move 5 clips · 9.4 GB**, **Rename 5
clips**. It's disabled, and
says why, when there's nothing to do: *All 12 clips are already in place*, or
*No D-Log, D-Log 2 or D-Log M clips here*.

Sizes are shown the way Explorer shows them (1 KB = 1024 bytes).

### 4.10 Remembered options

Separate by, the prefixes, Copy/Move, Include subfolders, Bring sidecars
along, Together/Per folder, Verify copies and the theme are remembered between
runs. The destination isn't (§4.4). An option that's missing or unreadable
falls back to its default on its own.

---

## 5. Sorting

Only log clips are touched. Everything else — other profiles, *No profile*,
*Unreadable* — stays where it is. D-Sep sorts the clips of the last scan, so
every sort needs a scan first; **Sort again** (§6) rescans.

### 5.1 Never overwrites

- **Already in place** — a clip whose target name already holds a file of the
  same size and last-modified time (within 2 seconds, as FAT-formatted cards
  round file times) is taken to be a copy of it and skipped. The size alone
  wouldn't do: Windows sizes a copy up front, so one cut short already has the
  full size.
- **Taken names** — a *different* file with the same name means the clip gets
  ` (2)`, ` (3)`… instead: the first number that's free for the clip *and* all
  its sidecars. With **Together**, this is what happens to `DJI_0001.MP4`
  from both `100MEDIA` and `101MEDIA`.
- Nothing is ever replaced, even if a file appears at the target while the
  sort runs.

### 5.2 Safe copies

- **Temporary name** — a copy is written as `<name>.d-sep-partial` and
  renamed only once it's complete (and verified, if that's on), so a crash or
  a power cut never leaves a half-written clip under its real name. A leftover
  partial file is deleted the next time that clip is copied.
- **Moves across drives** write the copy straight to the disk rather than to
  Windows' cache, and flush it and its name to the disk before the original is
  deleted. If the original can't be deleted, the clip is reported as *Copied,
  but the original couldn't be removed*.
- **What D-Sep deletes** — only its own partial copies, the folders it made
  and left empty (§5.4), and the original of a move to another drive once its
  copy is safely on the disk.
- **Long paths** — paths longer than Windows' old 260-character limit work.

### 5.3 Progress

The controls give way to a progress panel:

- **What's happening** — *Copying 3 of 12*, *Moving*, *Renaming*, *Moving
  back* (§7), or *Verifying* while a copy is read back.
- **How much** — bytes done of the total, or a percentage when verifying (the
  bytes would add up to twice the clips' size).
- **Speed and time left** — when bytes are really copied, from the last few
  seconds of progress: *· 410 MB/s · about 3 min left*. A rename has neither.
- **The file** in progress, cut from the start so its own name stays in view,
  and a **Cancel** button.
- **The list** scrolls to keep the clip being sorted in view, and marks the
  ones done.
- **Taskbar** — the window's taskbar button shows the progress too, and
  flashes when the work ends if D-Sep isn't the window in front.

### 5.4 Cancel

**Cancel** (it reads **Stopping…** until the work stops) leaves the clip in
progress as it was: its partial copy is deleted, mid-file if need be. Clips
already placed stay placed, with their sidecars. Folders D-Sep made that are
still empty — their clips failed, or never came — are removed again.

### 5.5 Closing the window mid-sort

Closing the window while a sort or an undo runs asks first, in a Windows
dialog: **Stop and quit?** *Files that aren't done yet are left as they
were.* **Keep going** is the first button and the default, and Escape keeps
going too. **Stop and quit** cancels the work as Cancel does, and closes D-Sep
once it has stopped.

---

## 6. The result screen

- **Heading** — *Done*, *Done, with problems*, *Stopped*, or *Nothing to do*
  when every clip was already in place.
- **What happened**, in a line: *Copied 12 clips into folders in Footage*,
  *Moved and renamed 5 clips into folders in Project*, *Renamed 5 clips in
  DCIM*. If nothing was placed, why: *3 clips were already in place (same size
  and date), so they were skipped.*
- **Profile cards** with the number of clips placed. A card opens its folder
  in Explorer when all of that profile's clips went into one folder.
- **Notes**, as they apply: how many sidecar files went along (or were added
  next to clips already in place), clips already in place, *Every copy was
  read back and checked against its original.*, and *Stopped early.
  Unfinished clips were left untouched.*
- **Failures** — *Couldn't copy 2 files*, with each file's path under the
  scanned folder and the reason. A sidecar that failed is listed on its own.
- **Buttons** — **Open folder** (where the clips went), **Undo** (after a
  move, §7), **Sort again** (scans the same folder again; Undo won't be
  offered after that, and its tooltip says so) and **Choose another folder…**.

---

## 7. Undo

After a move — including a rename in place — the result screen offers
**Undo**, which moves every file back where it was. It's available until the
next scan, and until D-Sep is closed: what to undo is kept in memory only.

- Files go back newest first, through the same safe transfer as a sort (§5.2),
  so never over a file that's there now, and verified when the sort was.
- The `D-Log` / `D-Log 2` / `D-Log M` folders the sort made are removed once
  they're empty.
- Progress, Cancel and closing the window work as in a sort (§5.3–§5.5).
- **When some files can't go back** (one is open in an editor, say), the rest
  are put back, D-Sep says which failed and why, and **Undo** stays on offer
  to try again. A file that has disappeared isn't tried again.
- **When everything is back**, D-Sep scans the folder again and says *Moved 12
  files back.*

A copy has nothing to undo: the originals are where they were.

---

## 8. Menu, Help and About

The cog in the top-right corner opens a menu:

- **About** — the logo, the version, *© 2026 ZTCS · MIT License*, the credit
  for the profile key, and links to **GitHub** and **Third-party licenses**
  (the open-source software D-Sep is built with, and their licenses, shipped
  with the app).
- **GitHub** — opens the project page in the default browser.
- **Help** — *How to use D-Sep*: four steps (choose a folder, pick how to
  separate, copy or move, start) and a few tips.
- **Theme** — Light, Dark or Auto (§9). Picking one leaves the menu open, so
  the change can be seen and undone.

Help and About open in a sheet over the window; Escape, the close button or a
click outside closes it.

The **?** buttons beside Include subfolders, Bring sidecars along and Verify
copies explain them on click; hovering the option says the same.

---

## 9. Appearance

- **Theme** — Light, Dark, or Auto (the default), which follows Windows. It
  colors the title bar too. The window opens in the saved theme, never white
  first and then dark.
- **Profile colors** — D-Log is blue, D-Log 2 orange and D-Log M magenta,
  across the stats, badges, prefixes and result cards.
- **Contrast** — the colors of both themes meet WCAG AA: 4.5:1 for text, 3:1
  for what marks a control.
- **Reduced motion** — with Windows' animation effects off, the button and
  menu animations are too.
- **Window** — 600 × 760 by default, at least 600 × 640, titled *D-Sep* and
  its version. In a short window the stat cards get smaller to leave room for
  the list, and the controls stay in view.
- **Right-click** — off outside text fields, which keep Cut, Copy and Paste.

---

## 10. Keyboard and accessibility

- `Tab` / `Shift + Tab` — move between controls. In the radio groups
  (Separate by, Copy/Move, Subfolders) only the chosen option is a Tab stop.
- Arrow keys, `Home`, `End` — change the choice in a radio group; move
  through the menu, the theme buttons included.
- `Enter` / `Space` — press the focused button, or open the folder picker
  from the drop area.
- `Esc` — close the menu (focus goes back to the cog) or a sheet.

After a scan, a sort or an undo, focus goes back to where it was if that's
still there, or to the clip list or the result heading, so keyboard users
don't land back at the top of the window. Screen readers hear each file of a
sort once (and again when it's verified), errors as alerts, and the progress
bar's value.

---

## 11. Storage and privacy posture

- **No network** — D-Sep makes no network requests: no telemetry, no update
  check. **GitHub** hands a fixed address to the default browser. The window's
  content security policy allows no remote origins and no `eval`.
- **What's stored** — only the remembered options (§4.10), in WebView2's
  storage in the app's data folder for your Windows account. The undo record
  (§7) lives in memory and is gone when D-Sep closes.
- **The UI can't name files to move** — a sort works only on the clips of the
  last scan, which the backend keeps itself, and D-Sep opens only the folders
  it scanned or sorted into.
- **Safe to run from a card** — D-Sep starts Explorer by its full path, with
  the folder quoted, and loads the Windows DLLs it uses from System32, so
  files placed next to a portable copy can't run in its place.

---

## 12. Platform support and installation

- **Windows 10 and 11, 64-bit.** D-Sep uses the WebView2 runtime that comes
  with Windows; the installer adds it if it's missing.
- **`D-Sep_<version>_x64-setup.exe`** — installs D-Sep for your Windows
  account, with no admin prompt, and adds it to the Start menu. Uninstall it
  from **Settings → Apps**.
- **`D-Sep_<version>_x64-portable.exe`** — the whole app in one file, with
  nothing to install and the Visual C++ runtime built in. Kept with your
  clips, it offers to sort the folder it's in (§2.1).
- **Not code-signed** — the first time either file runs, SmartScreen may say
  *Windows protected your PC*: click **More info**, then **Run anyway**.
- **Updates** — there's no in-app updater. New versions, with their release
  notes, are on the
  [Releases](https://github.com/ztcs-software/d-sep/releases) page. Versions
  with a suffix (`-beta-1`, `-rc-1`) are pre-releases.
