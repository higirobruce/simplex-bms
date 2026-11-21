import { beforeEach } from "vitest";
import { assertTestDatabaseUrl } from "./guard";

// Runs for every test file. With no DATABASE_URL (no .env.test) the database is skipped
// silently so database-free tests still run. Otherwise it must be bms_test, and every test
// starts from an empty database whether or not the file imports ./db.
if (process.env.DATABASE_URL) {
  assertTestDatabaseUrl(process.env.DATABASE_URL);
  beforeEach(async () => {
    const { resetDb } = await import("./db");
    await resetDb();
  });
}
