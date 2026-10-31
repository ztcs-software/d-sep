import { describe, it, expect } from "vitest";
import type { Clip } from "./api";
import { DEFAULT_PREFIXES } from "./options";
import { newPrefix, targetOf, type Placing } from "./target";

const root = "E:\\DCIM";

/** A clip of `kind` at `name`, as a scan of `folder` lists it. */
function clip(name: string, kind: "dlog" | "dlog2" | "dlogm" | "unknown" = "dlog", folder = root): Clip {
  return { path: `${folder}\\${name}`, name, size: 1, profile: { kind }, sidecars: [], error: null };
}

/** Folders, Together, into the scanned folder, unless `changed` says otherwise. */
function placing(changed: Partial<Placing> = {}): Placing {
  return { root, destination: null, action: "folders", layout: "gather", prefixes: DEFAULT_PREFIXES, ...changed };
}

// These mirror Job::new and output_dir in sort.rs, and its tests: where the UI says a clip goes is where it goes.
describe("targetOf", () => {
  it("puts clips into profile folders in the scanned folder, or in each subfolder", () => {
    expect(targetOf(clip("A.MP4"), placing())).toBe("D-Log\\A.MP4");
    expect(targetOf(clip("sub\\E.MP4"), placing())).toBe("D-Log\\E.MP4");
    expect(targetOf(clip("sub\\E.MP4", "dlog2"), placing({ layout: "each" }))).toBe("sub\\D-Log 2\\E.MP4");
  });

  it("renames clips in their own folders, and leaves clips without a log profile alone", () => {
    expect(targetOf(clip("sub\\E.MP4"), placing({ action: "rename", layout: "each" }))).toBe("sub\\DL_E.MP4");
    expect(targetOf(clip("C.MP4", "dlogm"), placing({ action: "both" }))).toBe("D-Log M\\DLM_C.MP4");
    expect(targetOf(clip("D.MP4", "unknown"), placing({ action: "both" }))).toBe("");
  });

  it("keeps each clip's subfolder under a destination", () => {
    const into = placing({ destination: "F:\\Project", action: "rename" });
    expect(targetOf(clip("sub\\E.MP4"), into)).toBe("sub\\DL_E.MP4");
  });

  it("leaves clips already in a destination inside the scanned folder where they are, whatever its case", () => {
    expect(targetOf(clip("Out\\D-Log\\A.MP4"), placing({ destination: "e:\\dcim\\Out", layout: "each" }))).toBe(
      "D-Log\\A.MP4",
    );
    expect(targetOf(clip("Out\\DL_A.MP4"), placing({ destination: "E:\\DCIM\\Out", action: "rename" }))).toBe("DL_A.MP4");
  });

  it("sorts a profile folder into itself and the folders beside it, never into D-Log\\D-Log", () => {
    const own = "E:\\DCIM\\D-Log";
    expect(targetOf(clip("A.MP4", "dlog", own), placing({ root: own }))).toBe("A.MP4");
    expect(targetOf(clip("B.MP4", "dlog2", own), placing({ root: own }))).toBe("E:\\DCIM\\D-Log 2\\B.MP4");
  });
});

describe("newPrefix", () => {
  it("is the profile's prefix, unless the name already starts with it, ignoring case", () => {
    expect(newPrefix(clip("E.MP4", "dlog2"), placing({ action: "rename" }))).toBe("DL2_");
    expect(newPrefix(clip("dl_E.MP4"), placing({ action: "rename" }))).toBe("");
  });

  it("is empty when sorting into folders only", () => {
    expect(newPrefix(clip("E.MP4"), placing())).toBe("");
  });
});
