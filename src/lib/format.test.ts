import { describe, it, expect } from "vitest";
import { dropNotice, extension, formatBytes, formatDuration, joinPath, plural, sameFolder, splitPath } from "./format";

describe("formatBytes", () => {
  it("shows whole bytes below 1 KB", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    // A rate in bytes per second is rarely whole.
    expect(formatBytes(512.333)).toBe("512 B");
  });

  it("counts 1024 bytes to the KB, like Explorer, with one decimal below 10", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(10 * 1024)).toBe("10 KB");
    expect(formatBytes(250 * 1024 ** 2)).toBe("250 MB");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
  });

  it("stops at TB", () => {
    expect(formatBytes(2048 * 1024 ** 4)).toBe("2048 TB");
  });
});

describe("plural", () => {
  it("adds an s to anything but one", () => {
    expect(plural(0, "clip")).toBe("0 clips");
    expect(plural(1, "clip")).toBe("1 clip");
    expect(plural(2, "clip")).toBe("2 clips");
  });
});

describe("splitPath", () => {
  it("splits a folder into its name and parent", () => {
    expect(splitPath("C:\\Footage\\Day 1")).toEqual({ name: "Day 1", parent: "C:\\Footage" });
    expect(splitPath("D:/DCIM/100MEDIA")).toEqual({ name: "100MEDIA", parent: "D:/DCIM" });
  });

  it("ignores trailing separators", () => {
    expect(splitPath("C:\\Footage\\")).toEqual({ name: "Footage", parent: "C:" });
  });

  it("keeps a drive root as the name", () => {
    expect(splitPath("E:\\")).toEqual({ name: "E:", parent: "" });
  });
});

describe("extension", () => {
  it("is the upper-cased text after the last dot of the file name", () => {
    expect(extension("C:\\Footage\\DJI_0001.lrf")).toBe("LRF");
    expect(extension("DJI_0001.MP4.SRT")).toBe("SRT");
  });

  it("is empty for names without one, and for dotfiles", () => {
    expect(extension("C:\\Footage.v2\\README")).toBe("");
    expect(extension("C:\\Footage\\.hidden")).toBe("");
  });
});

describe("formatDuration", () => {
  it("rounds to seconds under a minute, never showing 0", () => {
    expect(formatDuration(0.2)).toBe("1 s");
    expect(formatDuration(44.6)).toBe("45 s");
  });

  it("rounds to minutes under an hour", () => {
    expect(formatDuration(59.6)).toBe("1 min");
    expect(formatDuration(150)).toBe("3 min");
  });

  it("shows hours and the minutes left over", () => {
    expect(formatDuration(3600)).toBe("1 h");
    expect(formatDuration(4800)).toBe("1 h 20 min");
  });
});

describe("sameFolder", () => {
  it("ignores case and trailing separators", () => {
    expect(sameFolder("C:\\Footage\\", "c:\\footage")).toBe(true);
    expect(sameFolder("E:\\", "E:")).toBe(true);
    expect(sameFolder("C:\\Footage", "C:\\Footage 2")).toBe(false);
  });
});

describe("dropNotice", () => {
  const root = "E:\\DCIM\\100MEDIA";

  it("says nothing about a dropped folder", () => {
    expect(dropNotice([root], root)).toBe("");
    expect(dropNotice(["E:\\"], "E:\\")).toBe("");
  });

  it("says that dropped files show all of their folder", () => {
    expect(dropNotice([`${root}\\DJI_0001.MP4`], root)).toBe("Showing everything in 100MEDIA, where the dropped file is.");
    expect(dropNotice([`${root}\\DJI_0001.MP4`, `${root}\\DJI_0002.MP4`], root)).toBe(
      "Showing everything in 100MEDIA, where the dropped files are.",
    );
    expect(dropNotice(["E:\\DJI_0001.MP4"], "E:\\")).toBe("Showing everything in E:, where the dropped file is.");
  });

  it("says that only the first of several folders is scanned", () => {
    expect(dropNotice([root, "E:\\DCIM\\101MEDIA"], root)).toBe("One folder at a time: this is the first of the 2 you dropped.");
    expect(dropNotice([`${root}\\A.MP4`, "F:\\B.MP4"], root)).toBe("One folder at a time: this is the first of the 2 you dropped.");
  });
});

describe("joinPath", () => {
  it("joins with one backslash, whatever the folder ends with", () => {
    expect(joinPath("C:\\Footage", "D-Log")).toBe("C:\\Footage\\D-Log");
    expect(joinPath("E:\\", "D-Log")).toBe("E:\\D-Log");
    expect(joinPath("D:/Card/", "D-Log 2")).toBe("D:/Card\\D-Log 2");
  });
});
