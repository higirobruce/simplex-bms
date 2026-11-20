import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { expect, test } from "vitest";

const DEAD = ["Workbench", "Hardware Operations", "hardware.shop",
              "HW-OPS", "workshop", "Higiro Hardware"];

function walk(dir: string, exts: string[]): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) out.push(...walk(full, exts));
    else if (exts.includes(path.extname(full))) out.push(full);
  }
  return out;
}

test("no Workshop Steel vocabulary survives in src/", () => {
  const found: string[] = [];
  for (const file of walk("src", [".ts", ".tsx", ".css"])) {
    const body = readFileSync(file, "utf8");
    for (const term of DEAD) {
      if (body.toLowerCase().includes(term.toLowerCase())) found.push(`${file}: ${term}`);
    }
  }
  expect(found).toEqual([]);
});
