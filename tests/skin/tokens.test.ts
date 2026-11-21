import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { contrastRatio, readRootTokens } from "../helpers/css";

const tokens = readRootTokens("src/app/globals.css");

test("palette matches the Cloud & Azure spec", () => {
  expect(tokens.ink).toBe("#0F1B2A");
  expect(tokens.accent).toBe("#176596");
  expect(tokens["hi-vis"]).toBe("#3FA9E5");
  expect(tokens.bg).toBe("#F1F6FB");
  expect(tokens.surface).toBe("#FFFFFF");
});

test("radius is 14px", () => {
  expect(tokens.radius).toBe("14px");
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
  for (const name of ["--font-sans", "--font-display", "--font-mono"]) {
    expect(layout).toMatch(new RegExp(`variable:\\s*["']${name}["']`));
  }
});

test("no retired skin colour survives in the token block", () => {
  const values = Object.values(tokens).map((v) => v.toUpperCase());
  const dead = [
    "#ECEAE3", "#1A1B1E", "#E8651F", "#D2CFC6", // Workshop Steel
    "#121212", "#E01B20", "#FFD400", "#F3F3EF", "#C2141A", // Shelf Edge
  ];
  for (const colour of dead) expect(values).not.toContain(colour);
});

test("body text on canvas and on surface meets WCAG AA", () => {
  expect(contrastRatio(tokens.ink, tokens.bg)).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(tokens.ink, tokens.surface)).toBeGreaterThanOrEqual(4.5);
});

// The two azures carry different contracts, and swapping them is the one way to
// break this palette: --accent is dark enough for white text, --hi-vis is not.
test("the bright azure is a ground for dark text, never text on a light surface", () => {
  expect(contrastRatio(tokens["hi-vis"], tokens.ink)).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(tokens["hi-vis"], tokens.surface)).toBeLessThan(3);
});

test("white text on the accent fill meets AA (filled buttons, pills and chips)", () => {
  expect(contrastRatio(tokens.surface, tokens.accent)).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(tokens.surface, tokens["accent-hover"])).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(tokens.surface, tokens.signal)).toBeGreaterThanOrEqual(4.5);
});

// line-strong is deliberately excluded: it is a border and scrollbar thumb, not text,
// so WCAG non-text contrast (3:1) applies rather than 4.5:1.
test("every text-bearing foreground token meets AA on every light ground", () => {
  const foregrounds = ["ink", "ink-soft", "ink-mute", "gold", "danger", "success", "warning", "info"];
  const grounds = ["bg", "bg-tint", "surface", "surface-2", "surface-3", "accent-soft"];
  for (const fg of foregrounds) {
    for (const ground of grounds) {
      const ratio = contrastRatio(tokens[fg], tokens[ground]);
      expect(ratio, `${fg} on ${ground} = ${ratio.toFixed(2)}`).toBeGreaterThanOrEqual(4.5);
    }
  }
});

test("each status token meets AA on its own -soft tint (badge pattern)", () => {
  for (const name of ["gold", "danger", "success", "warning", "info"]) {
    const ratio = contrastRatio(tokens[name], tokens[`${name}-soft`]);
    expect(ratio, `${name} on ${name}-soft = ${ratio.toFixed(2)}`).toBeGreaterThanOrEqual(4.5);
  }
});

// Non-text contrast (WCAG 1.4.11): a 3:1 floor for borders and scrollbar thumbs,
// deliberately separate from the 4.5:1 text assertions above.
test("line-strong border meets the 3:1 non-text floor on bg", () => {
  expect(contrastRatio(tokens["line-strong"], tokens.bg)).toBeGreaterThanOrEqual(3);
});
