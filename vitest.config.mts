import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";
import { existsSync, readFileSync } from "node:fs";

// Test env comes from .env.test only. If it is missing, DATABASE_URL is forced empty so a
// stray .env can never point the tests at the dev database.
const testEnv: Record<string, string> = {};
if (existsSync(".env.test")) {
  for (const line of readFileSync(".env.test", "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Z0-9_]+)\s*=\s*"?([^"\n]*)"?\s*$/);
    if (m) testEnv[m[1]] = m[2];
  }
}

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    globalSetup: ["./tests/helpers/setup-db.ts"],
    setupFiles: ["./tests/helpers/setup-each.ts"],
    env: { ...testEnv, DATABASE_URL: testEnv.DATABASE_URL ?? "" },
    fileParallelism: false,
  },
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
});
