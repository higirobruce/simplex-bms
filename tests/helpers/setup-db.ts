import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { PrismaClient } from "@prisma/client";

/** Parse KEY="value" lines from .env.test. Returns null if the file is absent. */
export function readEnvTest(): Record<string, string> | null {
  const file = resolve(process.cwd(), ".env.test");
  if (!existsSync(file)) return null;
  const out: Record<string, string> = {};
  for (const line of readFileSync(file, "utf8").split("\n")) {
    const m = line.match(/^\s*([A-Z0-9_]+)\s*=\s*"?([^"\n]*)"?\s*$/);
    if (m) out[m[1]] = m[2];
  }
  return out;
}

/**
 * Vitest globalSetup. Creates bms_test if absent and applies migrations.
 *
 * Deliberately soft-fails: if .env.test is missing or Postgres is down it warns and
 * returns, so database-free tests (skin, qty) still run. Database tests then fail
 * loudly on their own (db.ts refuses to connect to anything but bms_test).
 */
export default async function setup() {
  const env = readEnvTest();
  const url = env?.DATABASE_URL;
  if (!url) {
    console.warn("[setup-db] .env.test missing or has no DATABASE_URL; skipping test database setup.");
    return;
  }
  const target = new URL(url);
  const dbName = decodeURIComponent(target.pathname.slice(1));
  if (dbName !== "bms_test") {
    throw new Error(`[setup-db] refusing to set up "${dbName}"; .env.test must point at bms_test.`);
  }

  // CREATE DATABASE cannot run on a connection to the database itself, so use "postgres".
  const admin = new URL(url);
  admin.pathname = "/postgres";
  const client = new PrismaClient({ datasourceUrl: admin.toString() });
  try {
    const found = await client.$queryRaw<unknown[]>`select 1 from pg_database where datname = ${dbName}`;
    if (found.length === 0) await client.$executeRawUnsafe(`create database "${dbName}"`);
  } catch (e) {
    console.warn(`[setup-db] could not check/create database "${dbName}" via the postgres admin database (${(e as Error).message.split("\n").pop()}); database tests will fail. Causes: server down, no access to "postgres", or no CREATEDB permission.`);
    return;
  } finally {
    await client.$disconnect().catch(() => {});
  }

  execFileSync("npx", ["prisma", "migrate", "deploy"], {
    stdio: "pipe",
    env: { ...process.env, ...env },
  });
}
