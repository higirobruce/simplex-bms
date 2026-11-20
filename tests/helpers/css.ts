import { readFileSync } from "node:fs";

/** Returns the --name: value declarations of the FIRST :root { ... } block, keys without "--". */
export function readRootTokens(cssPath: string): Record<string, string> {
  const css = readFileSync(cssPath, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const start = css.search(/:root\s*\{/);
  if (start === -1) return {};
  const open = css.indexOf("{", start);
  let depth = 0;
  let end = css.length;
  for (let i = open; i < css.length; i++) {
    if (css[i] === "{") depth++;
    else if (css[i] === "}" && --depth === 0) {
      end = i;
      break;
    }
  }
  const block = css.slice(open + 1, end);
  const tokens: Record<string, string> = {};
  for (const m of Array.from(block.matchAll(/--([a-zA-Z0-9_-]+)\s*:\s*([^;]+);/g))) {
    tokens[m[1]] = m[2].trim();
  }
  return tokens;
}

function luminance(hex: string): number {
  if (!/^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.test(hex)) {
    throw new Error(`Expected a #rgb or #rrggbb hex colour, got "${hex}"`);
  }
  const h = hex.slice(1);
  const full = h.length === 3 ? h.split("").map((c) => c + c).join("") : h;
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(full.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2.1 contrast ratio between two hex colours. */
export function contrastRatio(hexA: string, hexB: string): number {
  const [a, b] = [luminance(hexA), luminance(hexB)];
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
