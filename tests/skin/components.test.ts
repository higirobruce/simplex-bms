import { readFileSync } from "node:fs";
import { expect, test } from "vitest";

const css = readFileSync("src/app/globals.css", "utf8");

test("the price-label classes exist", () => {
  for (const cls of [".price-card", ".dept-chip", ".shelf-mark"]) {
    expect(css).toContain(cls);
  }
});

test("classes referenced by existing markup still exist", () => {
  for (const cls of [".shelf-mark", ".brushed", ".panel-flat", ".hairline", ".pill", ".eyebrow", ".eyebrow-accent", ".tabular"]) {
    expect(css).toContain(cls);
  }
  expect(css).toMatch(/\.panel\b(?!-)/);
});

test("hi-vis yellow is never a panel or surface background", () => {
  const panelish = css.match(/\.(panel|panel-flat|brushed|price-card)\s*\{[^}]*\}/g) ?? [];
  expect(panelish.length).toBeGreaterThan(0);
  for (const block of panelish) {
    expect(block).not.toMatch(/--hi-vis|#FFD400/i);
  }
});

test("no Workshop Steel gradients survive", () => {
  for (const hex of ["#2D3035", "#15161A", "#1F2125", "#0E0F12"]) {
    expect(css.toUpperCase()).not.toContain(hex);
  }
  expect(css).not.toContain("linear-gradient");
});

test("panels carry no shadow and the pill has no dot", () => {
  const panel = css.match(/\.panel\s*\{[^}]*\}/)?.[0] ?? "";
  expect(panel).not.toContain("box-shadow");
  expect(css).not.toContain(".pill::before");
});

test("tailwind shadows use the cool ink base, not warm brown", () => {
  const cfg = readFileSync("tailwind.config.ts", "utf8");
  expect(cfg).not.toContain("21,17,13");
});
