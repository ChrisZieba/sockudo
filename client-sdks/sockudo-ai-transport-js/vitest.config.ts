import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    coverage: {
      enabled: true,
      provider: "v8",
      // The whole package is measured, not just the leaf utilities. A gate
      // scoped to a handful of files reports 100% while instrumenting ~1.6% of
      // the source, which is how upstream drift went unnoticed.
      include: ["src/**/*.ts"],
      exclude: ["src/**/*.test.ts", "src/**/mocks.ts"],
      reporter: ["text", "lcov"],
      thresholds: {
        // Global floor: the measured baseline, rounded down. Raise as coverage
        // improves; never lower to make a change pass.
        branches: 66,
        functions: 74,
        lines: 76,
        statements: 76,
        // The original always-100% files keep their stricter contract.
        "src/{errors,event-emitter,logger,utils,constants}.ts": {
          branches: 100,
          functions: 100,
          lines: 100,
          statements: 100,
        },
      },
    },
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
