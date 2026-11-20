import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { contrastRatio, readRootTokens } from "../helpers/css";

const tokens = readRootTokens("src/app/globals.css");

test("palette matches the Shelf Edge spec", () => {
  expect(tokens.ink).toBe("#121212");
  expect(tokens.signal).toBe("#E01B20");
  expect(tokens["hi-vis"]).toBe("#FFD400");
  expect(tokens.bg).toBe("#F3F3EF");
  expect(tokens.surface).toBe("#FFFFFF");
});

test("radius is 2px", () => {
  expect(tokens.radius).toBe("2px");
});

test("every token tailwind.config.ts references is defined", () => {
  const config = readFileSync("tailwind.config.ts", "utf8");
  // font-* variables are excluded: next/font injects them on <body>, not :root.
  // They are covered by the layout.tsx test below.
  const referenced = Array.from(config.matchAll(/var\(--([a-z0-9-]+)\)/g))
    .map((m) => m[1])
    .filter((name) => !/^font-/.test(name));
  expect(referenced.length).toBeGreaterThan(10);
  for (const name of referenced) expect(tokens).toHaveProperty(name);
});

test("layout.tsx declares the font variables tailwind.config.ts relies on", () => {
  const layout = readFileSync("src/app/layout.tsx", "utf8");
  for (const name of ["--font-geist-sans", "--font-display", "--font-mono"]) {
    expect(layout).toMatch(new RegExp(`variable:\\s*["']${name}["']`));
  }
});

test("no Workshop Steel colour survives in the token block", () => {
  const values = Object.values(tokens).map((v) => v.toUpperCase());
  for (const dead of ["#ECEAE3", "#1A1B1E", "#E8651F", "#D2CFC6"]) {
    expect(values).not.toContain(dead);
  }
});

test("body text on canvas and on surface meets WCAG AA", () => {
  expect(contrastRatio(tokens.ink, tokens.bg)).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(tokens.ink, tokens.surface)).toBeGreaterThanOrEqual(4.5);
});

test("hi-vis yellow is only legible against ink, never against surface", () => {
  expect(tokens["hi-vis"]).toBeDefined();
  expect(contrastRatio(tokens["hi-vis"], tokens.ink)).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(tokens["hi-vis"], tokens.surface)).toBeLessThan(3);
});
