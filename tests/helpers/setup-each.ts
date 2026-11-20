import { beforeEach } from "vitest";

// Every test in a file that imports ./db starts from an empty database.
// Files that never import it (skin, qty) pay nothing and need no Postgres.
beforeEach(async () => {
  if ((globalThis as { __testDbLoaded?: boolean }).__testDbLoaded) {
    const { resetDb } = await import("./db");
    await resetDb();
  }
});
