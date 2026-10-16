import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  DEFAULT_PREFIXES,
  MAX_PREFIX,
  OPTIONS_KEY,
  cleanPrefix,
  isDevicePrefix,
  loadOptions,
  saveOptions,
  type Options,
} from "./options";

/** A localStorage stand-in holding one saved value. */
function storage(saved: string | null = null) {
  const items = new Map<string, string>();
  if (saved !== null) items.set(OPTIONS_KEY, saved);
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
  };
}

const failing = {
  getItem: (): string | null => {
    throw new Error("storage disabled");
  },
  setItem: () => {
    throw new Error("quota exceeded");
  },
};

const DEFAULTS: Options = {
  mode: "copy",
  action: "folders",
  recursive: true,
  sidecars: true,
  prefixes: DEFAULT_PREFIXES,
  theme: "auto",
  layout: "gather",
  verify: false,
};

describe("loadOptions", () => {
  it("gives the defaults when nothing is saved, the JSON is broken, or storage can't be read", () => {
    for (const saved of [null, "", "{", "null", "42", '"copy"', "[]"]) {
      expect(loadOptions(storage(saved)), `saved: ${saved}`).toEqual(DEFAULTS);
    }
    expect(loadOptions(failing)).toEqual(DEFAULTS);
  });

  it("falls back field by field, keeping the valid ones", () => {
    const saved = JSON.stringify({ mode: "teleport", action: "both", theme: 3, layout: "each", verify: "yes" });
    expect(loadOptions(storage(saved))).toEqual({ ...DEFAULTS, action: "both", layout: "each" });
  });

  it("cleans saved prefixes, and replaces one that cleans to nothing", () => {
    const saved = JSON.stringify({ prefixes: { dlog: "..Log/1_", dlog2: "$ \t", dlogm: 7 } });
    expect(loadOptions(storage(saved)).prefixes).toEqual({ dlog: "Log1_", dlog2: "DL2_", dlogm: "DLM_" });
  });
});

describe("saveOptions", () => {
  it("round-trips every option", () => {
    const changed: Options = {
      mode: "move",
      action: "rename",
      recursive: false,
      sidecars: false,
      prefixes: { dlog: "A_", dlog2: "B_", dlogm: "C_" },
      theme: "dark",
      layout: "each",
      verify: true,
    };
    // Every field differs from its default, so one that isn't saved or loaded can't pass unnoticed.
    for (const key of Object.keys(DEFAULTS) as (keyof Options)[]) {
      expect(changed[key], `pick a non-default ${key}`).not.toEqual(DEFAULTS[key]);
    }
    const store = storage();
    saveOptions(changed, store);
    expect(loadOptions(store)).toEqual(changed);
  });

  it("does nothing when storage can't be written", () => {
    expect(() => saveOptions(DEFAULTS, failing)).not.toThrow();
  });
});

// cleanPrefix runs as the prefix is typed, so whatever it lets through must also pass check_prefix in sort.rs,
// or the sort would only fail once it's started. The Rust rules are read from sort.rs, so tightening them there
// fails this test until the UI catches up.
describe("cleanPrefix agrees with check_prefix in sort.rs", () => {
  const sortRs = readFileSync(fileURLToPath(new URL("../../src-tauri/src/sort.rs", import.meta.url)), "utf8");
  const checkPrefix = /fn check_prefix[\s\S]*?\n\}/.exec(sortRs)?.[0] ?? "";

  it("cuts to the same length, counting characters as Rust does", () => {
    expect(Number(/const MAX_PREFIX_CHARS: usize = (\d+);/.exec(sortRs)?.[1])).toBe(MAX_PREFIX);
    expect(cleanPrefix("x".repeat(MAX_PREFIX + 5))).toBe("x".repeat(MAX_PREFIX));
    // Two UTF-16 units each: cutting at 16 units would leave half an emoji, which serde refuses.
    expect(cleanPrefix("😀".repeat(MAX_PREFIX + 5))).toBe("😀".repeat(MAX_PREFIX));
  });

  it("knows the device names is_device_name refuses", () => {
    expect(sortRs).toContain('["CON", "PRN", "AUX", "NUL"]');
    expect(sortRs).toContain('name.starts_with("COM") || name.starts_with("LPT")');
    for (const device of ["NUL.", "con.x", "Aux.", "PRN .", "COM1.", "lpt9.a"]) {
      expect(isDevicePrefix(device), device).toBe(true);
    }
    for (const fine of ["CON", "NULL.", "CONX.", "COM.", "COM10.", "AUX_", "DL_"]) {
      expect(isDevicePrefix(fine), fine).toBe(false);
    }
  });

  it("drops every character check_prefix forbids", () => {
    const forbidden = /r#"(.*?)"#\.contains\(c\)/.exec(checkPrefix)?.[1];
    expect(forbidden, "the forbidden-character list in check_prefix").toBe('<>:"/\\|?*');
    for (const c of forbidden!) expect(cleanPrefix(`a${c}b`), `U+${c.codePointAt(0)!.toString(16)}`).toBe("ab");
  });

  it("drops control characters (Rust's char::is_control: U+0000–U+001F and U+007F–U+009F)", () => {
    expect(checkPrefix).toContain("char::is_control");
    for (let code = 0; code <= 0x9f; code++) {
      if (code > 0x1f && code < 0x7f) continue;
      expect(cleanPrefix(`a${String.fromCodePoint(code)}b`), `U+${code.toString(16)}`).toBe("ab");
    }
  });

  it("drops a leading dot, $ or whitespace (Rust's char::is_whitespace)", () => {
    expect(checkPrefix).toMatch(/starts_with\(\['\.', '\$'\]\)/);
    expect(checkPrefix).toContain("starts_with(char::is_whitespace)");
    // Unicode White_Space, which is what char::is_whitespace tests.
    const whitespace = "\t\n\v\f\r \u0085             "
      + "    　";
    for (const lead of [".", "$", ...whitespace]) {
      expect(cleanPrefix(`${lead}DL_`), `U+${lead.codePointAt(0)!.toString(16)}`).toBe("DL_");
    }
    expect(cleanPrefix(". $.DL_")).toBe("DL_");
  });

  it("keeps an ordinary prefix as typed, and turns anything that isn't a string into an empty one", () => {
    expect(cleanPrefix("Log 1 - ")).toBe("Log 1 - ");
    expect(cleanPrefix(undefined)).toBe("");
    expect(cleanPrefix(42)).toBe("");
  });
});
