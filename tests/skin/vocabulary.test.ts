import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { expect, test } from "vitest";

const DEAD = ["Workbench", "Hardware Operations", "hardware.shop",
              "HW-OPS", "workshop", "Higiro Hardware", "Space Grotesk", "JetBrains Mono"];

// Atelier-era vocabulary; regexes with boundaries so "the house(s)" does not match "warehouse".
const DEAD_RE: [string, RegExp][] = [
  ["the house", /\bthe houses?\b/i],
  ["your first piece", /\byour first piece\b/i],
  ["Curate/Curating", /\bcurat(e|es|ed|ing)\b/i],
  ["Atelier", /\batelier\b/i],
  ["bespoke", /\bbespoke\b/i],
  ["concierge", /\bconcierge\b/i],
  ["clientele", /\bclientele\b/i],
];

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
  for (const file of walk("src", [".ts", ".tsx", ".css", ".svg"])) {
    const body = readFileSync(file, "utf8");
    for (const term of DEAD) {
      if (body.toLowerCase().includes(term.toLowerCase())) found.push(`${file}: ${term}`);
    }
    for (const [name, re] of DEAD_RE) {
      if (re.test(body)) found.push(`${file}: ${name}`);
    }
  }
  expect(found).toEqual([]);
});

const DEAD_HEX = ["#ECEAE3", "#1A1B1E", "#E8651F", "#D2CFC6", "#2D3035", "#15161A", "#1F2125"];

test("no Workshop Steel hex literal survives anywhere under src/", () => {
  const found: string[] = [];
  for (const file of walk("src", [".ts", ".tsx", ".css", ".svg", ".js", ".jsx", ".json", ".html"])) {
    const body = readFileSync(file, "utf8").toLowerCase();
    for (const hex of DEAD_HEX) {
      if (body.includes(hex.toLowerCase())) found.push(`${file}: ${hex}`);
    }
  }
  expect(found).toEqual([]);
});
