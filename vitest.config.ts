import { defineConfig } from "vitest/config";

// Standalone from vite.config.ts, so the Svelte plugin isn't loaded: no test renders a component. The tests
// exercise plain TS from src/lib, and repo-level invariants in tests/ that read files from both sides of the
// IPC boundary. The default `node` environment is enough for both.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts", "tests/**/*.test.ts"],
  },
});
