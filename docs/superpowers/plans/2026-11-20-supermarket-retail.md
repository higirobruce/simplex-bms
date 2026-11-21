# Simplex for Supermarkets — Phase 0 and Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire Workshop Steel for the Shelf Edge supermarket skin, and reshape the data model so it can represent grocery trade and till sales — the two phases of the design that nothing is waiting on.

**Architecture:** Both phases are pure Next.js + Prisma work inside the existing repository on `main`. Phase 0 swaps a CSS custom-property block, a font pair and nine files of trade vocabulary; it ships alone and is visible immediately. Phase 1 widens eight `Int` quantity columns to `Decimal`, adds grocery fields and models, and introduces the till models (`Lane`, `Shift`, `Sale`, `SaleLine`, `Tender`) without any till code consuming them yet. No Rust and no Tauri appear in this plan — see Scope.

**Tech Stack:** Next.js 14.2.35, React 18, Prisma 5.22 + PostgreSQL, Zod, TanStack Query 5, Tailwind 3, TypeScript 5. **Vitest is introduced by Task 1** — this repository currently has no test runner and no tests.

**Spec:** `docs/superpowers/specs/2026-11-20-supermarket-retail-design.md`

## Scope

The spec has six phases. This plan covers **Phase 0 and Phase 1 only**, because they are the two that are unblocked and each produces working, shippable software on its own.

**Phase 2 (the Tauri till) deliberately gets its own plan, later.** Its first action per the spec is verifying C9 against real pilot hardware, and that answer decides between Tauri and a pure-Rust UI — which in turn decides whether the Shelf Edge tokens this plan writes are reused at the lane or rebuilt. Planning Phase 2 in detail before §9 Q6 is answered would be writing on sand. Phases 3–5 depend on RRA answers (§9 Q1–Q5) and on company paperwork (§9 Q8).

## Global Constraints

Every task's requirements implicitly include these. Values are copied verbatim from the spec.

- **Tax type is a four-value set**: `A` (A-EX, exempt), `B` (B-18.00%), `C`, `D`. Not A/B. (spec C5b)
- **Payment method is RRA's code list**: `01 CASH`, `02 CREDIT`, `03 CASH/CREDIT`, `04 BANK CHECK`, `05 DEBIT&CREDIT CARD`, `06 MOBILE MONEY`. (spec C5b)
- **Mobile money is recorded, never confirmed, in v1.** `Tender.providerRef` and `Tender.confirmed` exist from Phase 1 and no v1 code path writes them. (spec C5b)
- **Quantity scale is `Decimal(12, 3)`** — three decimal places, enough for grams. Money stays `Decimal(12, 2)`.
- **Currency `RWF`, timezone `Africa/Kigali`** — existing `Org` defaults, unchanged.
- **Shelf Edge palette**: ink `#121212`, signal red `#E01B20`, hi-vis yellow `#FFD400`, canvas `#F3F3EF`, surface `#FFFFFF`.
- **Shelf Edge type**: Archivo (display/signage), IBM Plex Mono (all prices and codes).
- **Shelf Edge radius is `2px`** — crisper than Workshop Steel's 4px.
- **Yellow containment rule**: `#FFD400` may be used on pills and on a single dashboard tile. It must never be a panel background. (spec §6.3 trade-off)
- **Workshop Steel is retired on `main`**, not kept alongside. Simplex carries one skin at a time. (spec §6.3)
- **No multi-buy, mix-and-match or loyalty-conditional pricing.** v1 promotions are time-boxed price overrides plus percentage or amount discounts at line and basket level. (spec §10)
- **Till sales do not reuse `Invoice`.** (spec Phase 1)

## Review Focus

Input classes the spec implies but which no task's tests would otherwise exercise, most likely to bite first. Each has its test assigned to the task that owns the code.

1. **`Decimal + number` silently produces a string.** Verified in this repository: `new Prisma.Decimal("10") + 5` evaluates to the string `"105"`, and `"105" < 0` is `false`, so a negative-stock guard passes and Prisma accepts `"105"` as a valid Decimal write. Stock becomes 105 instead of 15 with no error anywhere. `*` and `-` coerce to number and are safe by accident; only `+` corrupts. → **Task 6.**
2. **Weighed quantity arriving with more precision than the column holds.** A scale reporting `0.7345` kg against `Decimal(12,3)` rounds, and rounding a quantity after the price has been computed makes the receipt total disagree with line × price. → **Task 7.**
3. **Opening-stock CSV import rejecting decimal quantities.** `src/lib/validations.ts:111` is `z.coerce.number().int().positive()`. A shop importing `12.5` kg of rice gets "Qty must be a positive whole number" with no indication that the product is weighed. → **Task 8.**
4. **A product with no tax code reaching a sale.** `taxTyCd` is new in Phase 1 and every one of the two live orgs' existing products predates it. A sale of an untaxed-by-accident product is a compliance fault, not a cosmetic one. → **Task 9.**
5. **Weight-embedded barcodes colliding with product barcodes.** A barcode scanned at the lane may encode a PLU plus a weight rather than identify a product, so a unique index on `barcode` must not be the only lookup path and must not reject two different weighed items that share a PLU prefix. → **Task 9.**

---

## File Structure

**Phase 0 — created**
- `vitest.config.ts` — test runner config, node environment, no DOM needed for these tests.
- `tests/skin/tokens.test.ts` — asserts every token Tailwind references exists in `globals.css`, that the palette matches the spec, and contrast ratios.
- `tests/skin/vocabulary.test.ts` — asserts no Workshop Steel vocabulary survives anywhere in `src/`.
- `tests/helpers/css.ts` — reads `globals.css` and returns a `Record<string, string>` of `:root` custom properties; computes WCAG contrast.

**Phase 0 — modified**
- `src/app/globals.css` — token block (lines 5–44) and component classes (lines 86–179) replaced.
- `src/app/layout.tsx` — font imports (lines 3, 14–26), metadata (lines 31–32), body class (line 44).
- `tailwind.config.ts` — `fontFamily` fallback names (lines 50–56); colour mapping is already variable-driven and needs no change.
- `src/app/auth/login/page.tsx`, `src/app/auth/signup/page.tsx`, `src/app/[slug]/dashboard/page.tsx`, `src/app/[slug]/settings/page.tsx`, `src/app/[slug]/products/page.tsx`, `src/components/sidenav.tsx` — trade vocabulary.

**Phase 1 — created**
- `src/lib/qty.ts` — decimal quantity arithmetic. The only place quantities are added.
- `tests/lib/qty.test.ts`
- `prisma/migrations/<timestamp>_qty_decimal/` — the widening migration, alone.
- `prisma/migrations/<timestamp>_grocery_fields/`, `_grocery_models/`, `_promotions/`, `_till_models/`
- `tests/schema/*.test.ts` — one per model group, asserting shape and constraints against a live test database.
- `scripts/seed-grocery.ts` — Rwandan supermarket department spread.

**Phase 1 — modified**
- `prisma/schema.prisma` — eight `Int` → `Decimal(12,3)`; `Product` grocery fields; five new grocery/pricing models; five new till models; two new enums.
- `src/lib/validations.ts` — quantity schemas.
- `src/app/api/[slug]/stock/adjust/route.ts:35` — the one addition site.
- `src/app/api/[slug]/stock/import/route.ts` — decimal-aware import.

---

## PHASE 0 — Shelf Edge

Independent of everything downstream. Ships on its own.

### Task 1: Test harness and the skin contract

The skin's failure mode is being half-swapped — a token renamed in `globals.css` but still referenced by `tailwind.config.ts` renders as `unset` and is invisible in review. These tests make that impossible. They are written first and fail, because Shelf Edge does not exist yet.

**Files:**
- Create: `vitest.config.ts`
- Create: `tests/helpers/css.ts`
- Create: `tests/skin/tokens.test.ts`
- Create: `tests/skin/vocabulary.test.ts`
- Modify: `package.json` (devDependencies, `test` and `test:watch` scripts)

**Interfaces:**
- Consumes: nothing.
- Produces: `readRootTokens(cssPath: string): Record<string, string>` and `contrastRatio(hexA: string, hexB: string): number` from `tests/helpers/css.ts`, used by Task 3.

- [ ] **Step 1: Install Vitest and add scripts**

```bash
npm install -D vitest
```

Take whatever major resolves; the config in Step 2 has been stable across Vitest 1–3.
In `package.json` scripts add `"test": "vitest run"` and `"test:watch": "vitest"`.

- [ ] **Step 2: Write `vitest.config.ts`**

`environment: "node"`, `include: ["tests/**/*.test.ts"]`, and an alias mapping `@` to `./src` so later tasks can import application modules.

- [ ] **Step 3: Implement `tests/helpers/css.ts`**

Two exports. `readRootTokens(cssPath: string): Record<string, string>` reads the file and returns every `--name: value` declaration found inside the first `:root { ... }` block, keys without the leading `--`. `contrastRatio(hexA: string, hexB: string): number` returns the WCAG 2.1 relative-luminance contrast ratio.

- [ ] **Step 4: Write the failing token tests**

```ts
// tests/skin/tokens.test.ts
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
  const referenced = [...config.matchAll(/var\(--([a-z0-9-]+)\)/g)].map((m) => m[1]);
  expect(referenced.length).toBeGreaterThan(10);
  for (const name of referenced) expect(tokens).toHaveProperty(name);
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
  expect(contrastRatio(tokens["hi-vis"], tokens.ink)).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(tokens["hi-vis"], tokens.surface)).toBeLessThan(3);
});
```

The last test encodes the spec's yellow-containment trade-off as an assertion: yellow is a background for dark text, never a foreground on white.

- [ ] **Step 5: Write the failing vocabulary test**

```ts
// tests/skin/vocabulary.test.ts
const DEAD = ["Workbench", "Hardware Operations", "hardware.shop",
              "HW-OPS", "workshop", "Higiro Hardware"];

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
```

`walk` is a local recursive directory reader in the test file.

- [ ] **Step 6: Run the tests to verify they fail**

Run: `npm test`
Expected: FAIL. Token tests fail on `#ECEAE3`/`#1A1B1E` still being present; the vocabulary test fails listing the nineteen sites in nine files.

- [ ] **Step 7: Commit**

```bash
git add vitest.config.ts tests package.json package-lock.json
git commit -m "test: add vitest and failing Shelf Edge skin contract tests"
```

### Task 2: Shelf Edge tokens and fonts

**Files:**
- Modify: `src/app/globals.css:5-44` (token block)
- Modify: `src/app/layout.tsx:3,14-26,31-32,44`
- Modify: `tailwind.config.ts:50-56`
- Test: `tests/skin/tokens.test.ts` (from Task 1)

**Interfaces:**
- Consumes: the token names `tailwind.config.ts` already maps — `bg`, `bg-tint`, `surface`, `surface-2`, `surface-3`, `ink`, `ink-soft`, `ink-mute`, `accent`, `accent-hover`, `accent-soft`, `gold`, `gold-soft`, `line`, `line-strong`, `success`, `warning`, `danger`, `info` and their `-soft` variants, `radius`, `radius-lg`, `radius-xl`.
- Produces: the same token names plus `--signal` and `--hi-vis`. **Keep every existing name.** Renaming `--gold` would touch dozens of call sites across the app for no benefit; it is re-pointed at the signal red instead, and the two new names are added for the places that need the palette explicitly.

- [ ] **Step 1: Replace the token block in `src/app/globals.css`**

Replace lines 5–44 with a `/* ===== SHELF EDGE — Supermarket Retail ===== */` block. Required values, from Global Constraints: `--bg: #F3F3EF`, `--surface: #FFFFFF`, `--ink: #121212`, `--signal: #E01B20`, `--hi-vis: #FFD400`, `--radius: 2px`. `--gold` and `--gold-soft` are re-pointed at the signal red and a light tint of it, so existing `text-gold` / `bg-gold` call sites become the price-tag accent rather than orange. Derive `--bg-tint`, `--surface-2`, `--surface-3`, `--ink-soft`, `--ink-mute`, `--line`, `--line-strong` from the spec's own samples: `#E8E8E2`, `#4A4A44`, `#76766E`, `#8A8A82`, `#D7D7D2`. Status colours keep their roles; `--danger` becomes `--signal` so "Out of stock" and "error" read the same red. `--radius-lg` and `--radius-xl` scale from 2px.

- [ ] **Step 2: Swap the font pair in `src/app/layout.tsx`**

Line 3 becomes `import { Archivo, IBM_Plex_Mono } from "next/font/google";`. Replace the `Space_Grotesk` call with `Archivo` keeping `variable: "--font-display"` and weights `["400","500","600","700"]`; replace `JetBrains_Mono` with `IBM_Plex_Mono` keeping `variable: "--font-mono"` and weights `["400","500","600"]`. Update the local `const` names and the template literal on line 44. Leave `geistSans` alone — it is the body face and is not part of this skin change.

- [ ] **Step 3: Update metadata copy**

Line 31 becomes `` title: `${platformName} — The Floor` ``; line 32 becomes `description: "Supermarket retail operations"`.

- [ ] **Step 4: Update font fallback names in `tailwind.config.ts:50-56`**

Replace the three `"Space Grotesk"` fallbacks with `"Archivo"` and the `"JetBrains Mono"` fallback with `"IBM Plex Mono"`. The `colors` block is entirely `var(--…)` driven and must not change.

- [ ] **Step 5: Run the token tests**

Run: `npm test -- tests/skin/tokens.test.ts`
Expected: PASS, all six tests.

- [ ] **Step 6: Verify the app still builds**

Run: `npx tsc --noEmit && npm run build`
Expected: exit 0. A missing font export from `next/font/google` fails here, not at runtime.

- [ ] **Step 7: Commit**

```bash
git add src/app/globals.css src/app/layout.tsx tailwind.config.ts
git commit -m "feat(skin): replace Workshop Steel tokens with Shelf Edge palette and type"
```

### Task 3: Component classes — the price-label motif

The spec's design language is the shelf price label. Workshop Steel's component classes express the opposite idea (brushed metal, stencil, gradients) and have to be replaced rather than recoloured.

**Files:**
- Modify: `src/app/globals.css:57-179` (`.font-serif`, `.font-mono`, and the `@layer components` block)
- Test: `tests/skin/components.test.ts`

**Interfaces:**
- Consumes: `readRootTokens` from `tests/helpers/css.ts`.
- Produces: CSS classes `.panel`, `.panel-flat`, `.hairline`, `.pill`, `.shelf-mark`, `.price-card`, `.dept-chip`, `.eyebrow`, `.eyebrow-accent`, `.tabular`. **`.stencil-block` and `.brushed` are kept as aliases** — they are referenced by `login/page.tsx` and `signup/page.tsx`, and Task 4 does not touch markup structure.

- [ ] **Step 1: Write the failing component tests**

```ts
// tests/skin/components.test.ts
const css = readFileSync("src/app/globals.css", "utf8");

test("the price-label classes exist", () => {
  for (const cls of [".price-card", ".dept-chip", ".shelf-mark"]) {
    expect(css).toContain(cls);
  }
});

test("classes referenced by existing markup still exist", () => {
  for (const cls of [".stencil-block", ".brushed", ".panel", ".pill", ".eyebrow"]) {
    expect(css).toContain(cls);
  }
});

test("hi-vis yellow is never a panel or surface background", () => {
  const panelish = css.match(/\.(panel|panel-flat|brushed|price-card)\s*\{[^}]*\}/g) ?? [];
  expect(panelish.length).toBeGreaterThan(0);
  for (const block of panelish) {
    expect(block).not.toMatch(/--hi-vis|#FFD400/i);
  }
});

test("no Workshop Steel gradients survive", () => {
  expect(css).not.toContain("#2D3035");
  expect(css).not.toContain("#15161A");
});
```

The third test is the spec's yellow-containment rule enforced at the component level, complementing Task 1's contrast version.

- [ ] **Step 2: Run to verify failure**

Run: `npm test -- tests/skin/components.test.ts`
Expected: FAIL on the missing price-label classes and on the surviving `#2D3035` gradient in `.stencil-block`.

- [ ] **Step 3: Rewrite the component layer in `src/app/globals.css`**

`.panel` and `.panel-flat` become flat white cards with a `--line` hairline and no shadow — a price label has no depth. `.price-card` is the motif: white, 2px radius, a heavy `--ink` bottom rule, prices set in `--font-mono` with `font-variant-numeric: tabular-nums`. `.dept-chip` is a small square-cornered chip taking its background from a `--dept` custom property the markup sets, for department colour coding. `.pill` loses the `::before` dot and becomes a solid uppercase label. `.shelf-mark` replaces the stencil brand mark: flat `--ink` block, `--surface` text, no gradient. `.stencil-block` and `.brushed` are retained so existing markup keeps working, and **each gets its own rule block** — do not comma-join them with `.shelf-mark` or `.panel`. Step 1's `panelish` regex matches single-selector blocks only; a comma-joined selector makes it match nothing and the test then fails on its own `length > 0` guard rather than on the thing it is checking. `.font-serif` and `.font-mono` keep their current structure with the new family names. Update `::selection` to use `--signal`.

- [ ] **Step 4: Run the component and token tests**

Run: `npm test -- tests/skin`
Expected: PASS for tokens and components; `vocabulary.test.ts` still fails.

- [ ] **Step 5: Commit**

```bash
git add src/app/globals.css tests/skin/components.test.ts
git commit -m "feat(skin): replace Workshop Steel components with the price-label motif"
```

### Task 4: Trade vocabulary

Nineteen strings across nine files. The spec calls the supermarket vocabulary *The Floor · Open till · Lines · Gaps on shelf*.

**Files:**
- Modify: `src/app/auth/login/page.tsx:75,84,94,107,131,134,148` and the `HW-OPS` tile
- Modify: `src/app/auth/signup/page.tsx:106,114,118-120,124,212,236,251` and the `HW-OPS` tile
- Modify: `src/app/[slug]/dashboard/page.tsx:183,195,199,202`
- Modify: `src/app/[slug]/settings/page.tsx:42`
- Modify: `src/app/[slug]/products/page.tsx:21`
- Modify: `src/components/sidenav.tsx:101`
- Test: `tests/skin/vocabulary.test.ts` (from Task 1)

**Interfaces:**
- Consumes: nothing.
- Produces: no code interface. Copy only; no markup structure changes, which is why Task 3 kept the class aliases.

- [ ] **Step 1: Replace the copy**

Exact replacements, left to right:

| File:line | From | To |
|---|---|---|
| `login:75` | `/ Workbench` | `/ The Floor` |
| `login:84` | `Hardware Operations` | `Supermarket Retail` |
| `login:88-92` | `Stock.` / `Sell.` / `Restock.` | `Scan.` / `Sell.` / `Restock.` |
| `login:94` | `A no-nonsense system for hardware shops — inventory across warehouses, sales orders, goods receipts, and the daily counter work.` | `Everything a supermarket runs on — shelf stock, the tills, suppliers, and what today actually took.` |
| `login:131` | `Workbench access.` | `Open the floor.` |
| `login:134` | `Enter your credentials to clock in.` | `Enter your credentials to start a shift.` |
| `login:148` | `you@hardware.shop` | `you@supermarket.rw` |
| `login` tile | `HW-OPS` | `RETAIL` |
| `signup:106` | `/ Workbench` | `/ The Floor` |
| `signup:114` | `New shop setup` | `New store setup` |
| `signup:118-120` | `Open the` / `shop.` | `Open the` / `store.` |
| `signup:124` | `Set up your workspace in a few minutes — products, warehouses, and the counter all wired up for you.` | `Set up your store in a few minutes — departments, shelf stock, and the tills all wired up for you.` |
| `signup:212` | `you@hardware.shop` | `you@supermarket.rw` |
| `signup:236` | `Higiro Hardware Co.` | `Kigali Fresh Market` |
| `signup:251` | `higiro-hardware` | `kigali-fresh` |
| `signup` tile | `HW-OPS` | `RETAIL` |
| `dashboard:195` | `Operations Overview` | `Trading Today` |
| `dashboard:199` | `The Workbench.` | `The Floor.` |
| `dashboard:202` | `Stock on hand, money in motion, and what needs your attention this shift.` | `What is on the shelf, what the tills took, and what needs you before close.` |
| `settings:42` | `The particulars of your workshop — organisation, people, and access.` | `The particulars of your store — organisation, people, and access.` |
| `products:21` | `WGT-001,Widget,Hardware,12000,9000,10` | `MLK-001,Inyange Milk 1L,Dairy,1200,950,24` |
| `sidenav:101` | `Workbench / {slug}` | `The Floor / {slug}` |

- [ ] **Step 2: Run the vocabulary test**

Run: `npm test -- tests/skin/vocabulary.test.ts`
Expected: PASS with an empty `found` array.

- [ ] **Step 3: Run the whole suite and typecheck**

Run: `npm test && npx tsc --noEmit`
Expected: all pass, exit 0.

- [ ] **Step 4: Commit**

```bash
git add src/app src/components
git commit -m "feat(skin): swap hardware vocabulary for supermarket trade language"
```

### Task 5: Visual verification

Tests prove the tokens and copy are right; they cannot tell you the skin looks like a supermarket. This is the one task with a human-judgement gate.

**Files:**
- Modify: `docs/superpowers/specs/2026-11-20-supermarket-retail-design.md` (mark Phase 0 done)

**Interfaces:**
- Consumes: the running application.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Start the dev server**

Run: `npm run dev`

- [ ] **Step 2: Look at four screens in a browser**

`/auth/login`, `/<slug>/dashboard`, `/<slug>/products`, `/<slug>/stock`. Confirm against Global Constraints: yellow appears on pills and at most one dashboard tile and on no panel; prices are set in IBM Plex Mono; corners are 2px; no orange survives anywhere.

- [ ] **Step 3: Check both live orgs render**

The two existing orgs carry real data. Confirm neither has unstyled or invisible text from a token that changed meaning — particularly anywhere `text-gold` was used as a foreground on white, which is now red rather than orange and must still meet contrast.

- [ ] **Step 4: Mark Phase 0 complete in the spec and commit**

```bash
git add docs/superpowers/specs/2026-11-20-supermarket-retail-design.md
git commit -m "docs: mark Phase 0 Shelf Edge skin complete"
```

---

## PHASE 1 — Grocery and retail data model

Schema and server work. No UI changes beyond what the quantity type forces. Task 6 ships before Task 7 on purpose: the arithmetic hazard is fixed while quantities are still integers, so the migration lands on code that is already safe. Task 6b was added during execution — see its own note.

### Task 6: Decimal quantity arithmetic

**Files:**
- Create: `src/lib/qty.ts`
- Create: `tests/lib/qty.test.ts`
- Modify: `src/app/api/[slug]/stock/adjust/route.ts:34-40`

**Interfaces:**
- Consumes: `Prisma.Decimal` from `@prisma/client`.
- Produces:
  - `type QtyLike = Prisma.Decimal | number | string`
  - `toQty(v: QtyLike): Prisma.Decimal`
  - `addQty(...values: QtyLike[]): Prisma.Decimal`
  - `subQty(a: QtyLike, b: QtyLike): Prisma.Decimal`
  - `isNegative(v: QtyLike): boolean`
  - `formatQty(v: QtyLike, isWeighed: boolean): string` — three decimals when weighed, none when each.
  - Tasks 7, 8 and 13 use these. **No quantity is added with `+` anywhere after this task.**

- [ ] **Step 1: Write the failing tests**

```ts
// tests/lib/qty.test.ts
import { Prisma } from "@prisma/client";
const D = (s: string) => new Prisma.Decimal(s);

test("the native + hazard this module exists to prevent", () => {
  // Documents verified runtime behaviour: Decimal + number yields a STRING.
  expect(typeof ((D("10") as unknown as number) + 5)).toBe("string");
  expect((D("10") as unknown as number) + 5).toBe("105");
});

test("addQty adds instead of concatenating", () => {
  expect(addQty(D("10"), 5).toString()).toBe("15");
  expect(addQty(D("0.734"), D("1.266")).toString()).toBe("2");
  expect(addQty("10", "5", 1).toString()).toBe("16");
});

test("subQty keeps three-decimal precision", () => {
  expect(subQty(D("1"), D("0.734")).toString()).toBe("0.266");
});

test("isNegative catches what a string comparison misses", () => {
  expect(isNegative(D("-0.001"))).toBe(true);
  expect(isNegative(addQty(D("10"), -25))).toBe(true);
  expect(isNegative(D("0"))).toBe(false);
});

test("formatQty distinguishes weighed from each", () => {
  expect(formatQty(D("0.734"), true)).toBe("0.734");
  expect(formatQty(D("3"), false)).toBe("3");
  expect(formatQty(D("3.000"), false)).toBe("3");
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npm test -- tests/lib/qty.test.ts`
Expected: FAIL, `addQty` is not defined. The first test passes immediately — it documents existing behaviour and is the reason for the module.

- [ ] **Step 3: Implement `src/lib/qty.ts`**

Every function routes through `Prisma.Decimal` methods (`.plus`, `.minus`, `.isNegative`, `.toFixed`) and never through JS operators.

- [ ] **Step 4: Fix the one addition site**

`src/app/api/[slug]/stock/adjust/route.ts:34-40`: `currentQty` becomes `toQty(current?.qty ?? 0)`, `newQty` becomes `addQty(currentQty, qty)`, and the guard becomes `if (isNegative(newQty))`. The error message uses `formatQty`. This is the only `+` on a quantity in the repository — confirmed by `grep -rn 'qty +\|+ qty\|qty +=' src scripts`.

- [ ] **Step 5: Run tests and typecheck**

Run: `npm test && npx tsc --noEmit`
Expected: PASS, exit 0.

- [ ] **Step 6: Commit**

```bash
git add src/lib/qty.ts tests/lib/qty.test.ts "src/app/api/[slug]/stock/adjust/route.ts"
git commit -m "feat: add decimal quantity arithmetic and remove the + coercion hazard"
```

### Task 6b: Database test harness

**Added during execution, not in the original plan.** Tasks 7 and 9–13 all create rows,
and the plan never said against which database. Run as written they would write test
fixtures into the local `bms` database where the two live orgs sit. This task provisions
a separate one. It goes after Task 6 because Task 6 is pure `Prisma.Decimal` arithmetic
and needs no database, and before Task 7 because Task 7 is the first task that does.

**Files:**
- Create: `.env.test` (gitignored, alongside the existing `.env`)
- Create: `tests/helpers/db.ts`
- Create: `tests/helpers/setup-db.ts`
- Modify: `vitest.config.ts` (`globalSetup`, and `setupFiles` for per-test cleanup)
- Modify: `package.json` (`test` script loads `.env.test`)
- Modify: `.gitignore` (add `.env.test`)

**Interfaces:**
- Consumes: the Vitest config from Task 1.
- Produces:
  - `prisma` — a `PrismaClient` bound to the test database, exported from `tests/helpers/db.ts`. Tasks 7 and 9–13 import this, **not** `@/lib/prisma`.
  - `resetDb(): Promise<void>` — truncates every application table, cascading, leaving the schema intact.
  - `seedOrg(): Promise<{ orgId: string; otherOrgId: string; userId: string; productId: string; locationId: string }>` — the minimal fixture Tasks 7 and 9–13 build on. Task 9's cross-org barcode test needs `otherOrgId`; Task 11's price-change test needs `userId`.

- [ ] **Step 1: Write the failing test**

```ts
// tests/helpers/db.test.ts
test("the test client is not pointed at the dev database", () => {
  expect(process.env.DATABASE_URL).toContain("bms_test");
  expect(process.env.DATABASE_URL).not.toMatch(/\/bms(\?|$)/);
});

test("resetDb leaves no rows behind", async () => {
  const { orgId } = await seedOrg();
  expect(await prisma.org.count()).toBeGreaterThan(0);
  await resetDb();
  expect(await prisma.org.count()).toBe(0);
  expect(await prisma.product.count()).toBe(0);
});

test("seedOrg produces two distinct orgs", async () => {
  const f = await seedOrg();
  expect(f.orgId).not.toBe(f.otherOrgId);
  const p = await prisma.product.findUnique({ where: { id: f.productId } });
  expect(p!.orgId).toBe(f.orgId);
});
```

The first test is the one that matters: it fails loudly if the harness is ever pointed
at the development database, which is the accident this whole task exists to prevent.

- [ ] **Step 2: Run to verify failure**

Run: `npm test -- tests/helpers/db.test.ts`
Expected: FAIL — `resetDb` is not defined.

- [ ] **Step 3: Create `.env.test` and gitignore it**

`DATABASE_URL="postgresql://<same credentials as .env>@localhost:5432/bms_test"`, plus the
same `NEXTAUTH_SECRET` and `NEXTAUTH_URL` the existing `.env` carries. Add `.env.test` to
`.gitignore`. **Never commit it.**

- [ ] **Step 4: Implement `tests/helpers/setup-db.ts` as Vitest `globalSetup`**

Creates the `bms_test` database if absent, then applies the schema with
`prisma migrate deploy` against `DATABASE_URL` from `.env.test`. Runs once per suite.

- [ ] **Step 5: Implement `tests/helpers/db.ts`**

Exports `prisma`, `resetDb` and `seedOrg` with the signatures in Interfaces. `resetDb`
discovers table names from `information_schema` rather than hardcoding a list, so Tasks
9–12 adding models does not require editing it. Register it as a `beforeEach` through
`setupFiles` so every test starts from an empty database.

- [ ] **Step 6: Run and verify isolation**

Run: `npm test` then, separately, confirm the dev database is untouched:
`psql bms -c 'select count(*) from "Org";'` — the count must match what it was before.

- [ ] **Step 7: Commit**

```bash
git add tests/helpers/db.ts tests/helpers/setup-db.ts tests/helpers/db.test.ts vitest.config.ts package.json .gitignore
git commit -m "test: provision an isolated test database harness"
```

### Task 7: Widen quantity columns to Decimal

The single riskiest change in the spec. It goes alone, in its own migration and its own commit, so it can be reverted without taking anything else with it.

**Files:**
- Modify: `prisma/schema.prisma:146,191,206,301,385,427,464,504`
- Create: `prisma/migrations/<timestamp>_qty_decimal/migration.sql`
- Create: `tests/schema/qty-decimal.test.ts`

**Interfaces:**
- Consumes: `addQty`, `toQty`, `formatQty` from Task 6.
- Produces: all eight columns typed `Decimal @db.Decimal(12, 3)`. Prisma returns these as `Prisma.Decimal` at every read site, which is what Task 8 then validates and Task 13 seeds.

The eight columns: `Product.reorderLevel` (146), `StockLevel.qty` (191), `StockMovement.qty` (206), `InvoiceLineItem.qty` (301), `SalesOrderLine.qty` (385), `DeliveryNoteLine.qty` (427), `PurchaseOrderLine.qty` (464), `GoodsReceiptLine.qty` (504).

- [ ] **Step 1: Write the failing test**

```ts
// tests/schema/qty-decimal.test.ts
test("a weighed quantity survives a round trip", async () => {
  const level = await prisma.stockLevel.create({
    data: { productId, locationId, qty: "0.734" },
  });
  expect(level.qty.toString()).toBe("0.734");
});

test("the column scale is 3, so a fourth decimal does not survive", async () => {
  const m = await prisma.stockMovement.create({
    data: { productId, toLocationId: locationId, qty: "1.2345", type: "RECEIPT", orgId },
  });
  expect(m.qty.toString()).not.toBe("1.2345");   // Review Focus #2
  expect(m.qty.decimalPlaces()).toBeLessThanOrEqual(3);
});

test("integer quantities still read as whole numbers", async () => {
  const line = await prisma.invoiceLineItem.create({ data: { ...base, qty: 3 } });
  expect(formatQty(line.qty, false)).toBe("3");
});
```

Review Focus #2 is pinned here. The assertion deliberately does not fix a rounding
direction — it fixes that a fourth decimal cannot survive silently. Task 8 then rejects
such a value at the API boundary so the caller finds out rather than the shelf.

- [ ] **Step 2: Run to verify failure**

Run: `npm test -- tests/schema/qty-decimal.test.ts`
Expected: FAIL — `qty: "0.734"` is rejected, or stored as `1`.

- [ ] **Step 3: Change the eight columns and generate the migration**

Each becomes `Decimal @db.Decimal(12, 3)`, keeping its existing `@default` where present (`StockLevel.qty @default(0)`, `InvoiceLineItem.qty @default(1)`, `Product.reorderLevel @default(10)`).

Run: `npx prisma migrate dev --name qty_decimal`

- [ ] **Step 4: Verify the generated SQL is a widening, not a rewrite**

Read `prisma/migrations/<timestamp>_qty_decimal/migration.sql`. Every statement must be `ALTER COLUMN ... SET DATA TYPE DECIMAL(12,3)`. `Int` → `Decimal` is value-preserving in PostgreSQL, so **no backfill is needed and none should appear.** If the file contains a `DROP`, stop and fix the schema rather than the SQL.

- [ ] **Step 5: Run the full suite against the migrated database**

Run: `npm test && npx tsc --noEmit`
Expected: PASS. The typecheck is the real gate here — it flags every site where a `Prisma.Decimal` now flows into something expecting `number`.

- [ ] **Step 6: Fix the type errors the widening surfaces**

Expected shape: `l.qty * l.unitPrice` sites keep working (`*` coerces to number and is safe), `ol.qty - ol.qtyReceived` sites keep working (`-` coerces), and display sites need `formatQty`. Use the Task 6 helpers; do not reintroduce `+`.

- [ ] **Step 7: Commit**

```bash
git add prisma/schema.prisma prisma/migrations tests/schema/qty-decimal.test.ts src
git commit -m "feat(db)!: widen eight quantity columns from Int to Decimal(12,3)"
```

### Task 8: Quantity validation

**Files:**
- Modify: `src/lib/validations.ts:111,187,214,252,273,301`
- Modify: `src/app/api/[slug]/stock/import/route.ts`
- Create: `tests/lib/validations.test.ts`

**Interfaces:**
- Consumes: `toQty` from Task 6.
- Produces: `QtySchema` and `SignedQtySchema` exported from `src/lib/validations.ts`, used by the six line schemas and by the CSV importer.

- [ ] **Step 1: Write the failing tests**

```ts
test("a weighed quantity is accepted", () => {
  expect(QtySchema.parse("0.734").toString()).toBe("0.734");
  expect(QtySchema.parse(12.5).toString()).toBe("12.5");
});

test("zero and negative are rejected for a line quantity", () => {
  expect(QtySchema.safeParse(0).success).toBe(false);
  expect(QtySchema.safeParse(-1).success).toBe(false);
});

test("a stock adjustment may be negative but not zero", () => {
  expect(SignedQtySchema.safeParse(-2.5).success).toBe(true);
  expect(SignedQtySchema.safeParse(0).success).toBe(false);
});

test("more than three decimals is rejected rather than silently rounded", () => {
  const r = QtySchema.safeParse("1.2345");
  expect(r.success).toBe(false);
});

test("the opening-stock importer accepts decimal quantities", () => {
  // Review Focus #3 — was z.coerce.number().int().positive()
  expect(StockImportRowSchema.safeParse(
    { sku: "RICE-5", location: "Main", qty: "12.5" }
  ).success).toBe(true);
});
```

The fourth test is a deliberate choice: rejecting excess precision at the API boundary is better than Task 7's database rounding, because the caller finds out. Task 7's test covers what happens if something bypasses this.

- [ ] **Step 2: Run to verify failure**

Run: `npm test -- tests/lib/validations.test.ts`
Expected: FAIL — `QtySchema` undefined; the importer test fails on `.int()`.

- [ ] **Step 3: Implement the schemas**

`QtySchema` is a `z.union` of string and number, coerced through `toQty`, refined to be positive and to have at most three decimal places, with the message `"Quantity must be positive, to at most 3 decimal places"`. `SignedQtySchema` is the same without the positivity refinement and with a non-zero refinement. Replace all six `z.number().int()` quantity schemas, including line 111's `z.coerce.number().int().positive()`.

- [ ] **Step 4: Make the importer decimal-aware**

`src/app/api/[slug]/stock/import/route.ts` uses the new schema and reports the row number with the quantity that failed.

- [ ] **Step 5: Run and commit**

Run: `npm test && npx tsc --noEmit`

```bash
git add src/lib/validations.ts "src/app/api/[slug]/stock/import/route.ts" tests/lib/validations.test.ts
git commit -m "feat: accept decimal quantities through validation and CSV import"
```

### Task 9: Product grocery fields

**Files:**
- Modify: `prisma/schema.prisma` (`Product` model, plus a `TaxType` enum)
- Create: `prisma/migrations/<timestamp>_grocery_fields/migration.sql`
- Modify: `src/lib/validations.ts` (product schema)
- Modify: `src/app/api/[slug]/products/route.ts`
- Create: `tests/schema/product-grocery.test.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Product` gains `barcode String?`, `plu String?`, `isWeighed Boolean @default(false)`, `taxTyCd TaxType @default(B)`, `department String @default("General")`, `packSize String?`, `shelfLifeDays Int?`. New `enum TaxType { A B C D }`. `@@unique([barcode, orgId])` and `@@index([plu, orgId])`. Tasks 10 and 12 reference `Product.taxTyCd` and `Product.isWeighed`.

**Decisions this task locks:**
- `taxTyCd` defaults to **`B`** (18%), not `A`. A product that is actually exempt and gets charged VAT is a visible pricing error someone reports; a product that should be taxed and silently is not is an under-declaration to RRA. Default to the safer direction and make exemption explicit. This is Review Focus #4.
- `department` is a plain `String` in v1, not a model, mirroring the existing `category`. A `Department` model with colour chips belongs with the UI that renders them, in a later phase.

- [ ] **Step 1: Write the failing tests**

```ts
test("tax type defaults to B rather than exempt", async () => {
  const p = await prisma.product.create({ data: { sku: "X1", name: "X", orgId } });
  expect(p.taxTyCd).toBe("B"); // Review Focus #4
});

test("the four RRA tax types are all accepted", async () => {
  for (const t of ["A", "B", "C", "D"] as const) {
    const p = await prisma.product.create({ data: { sku: `T${t}`, name: t, orgId, taxTyCd: t } });
    expect(p.taxTyCd).toBe(t);
  }
});

test("barcode is unique per org but not across orgs", async () => {
  await prisma.product.create({ data: { sku: "A", name: "A", orgId, barcode: "5901234123457" } });
  await expect(
    prisma.product.create({ data: { sku: "B", name: "B", orgId, barcode: "5901234123457" } })
  ).rejects.toThrow();
  const other = await prisma.product.create({
    data: { sku: "C", name: "C", orgId: otherOrgId, barcode: "5901234123457" },
  });
  expect(other.barcode).toBe("5901234123457");
});

test("two weighed products may share a PLU prefix", async () => {
  // Review Focus #5 — PLU is indexed, not unique; weight-embedded barcodes
  // are decoded to a PLU and must not be stored as product barcodes.
  await prisma.product.create({ data: { sku: "TOM", name: "Tomatoes", orgId, plu: "4664", isWeighed: true } });
  const second = await prisma.product.create({
    data: { sku: "TOMV", name: "Tomatoes Vine", orgId, plu: "46641", isWeighed: true },
  });
  expect(second.plu).toBe("46641");
});

test("barcode may be null for many products at once", async () => {
  await prisma.product.create({ data: { sku: "N1", name: "N1", orgId } });
  const n2 = await prisma.product.create({ data: { sku: "N2", name: "N2", orgId } });
  expect(n2.barcode).toBeNull();
});
```

The last test matters because PostgreSQL permits repeated NULLs in a unique index while some other databases do not; the two live orgs' existing products will all have a null barcode.

- [ ] **Step 2: Run to verify failure, then implement the schema and migrate**

Run: `npm test -- tests/schema/product-grocery.test.ts` → FAIL
Then: `npx prisma migrate dev --name grocery_fields`

- [ ] **Step 3: Extend the product Zod schema and API route**

Add the seven fields with `taxTyCd: z.enum(["A","B","C","D"])`, `barcode` optional and trimmed, `isWeighed` boolean. The products API accepts and returns them.

- [ ] **Step 4: Run and commit**

Run: `npm test && npx tsc --noEmit`

```bash
git add prisma src/lib/validations.ts "src/app/api/[slug]/products/route.ts" tests/schema/product-grocery.test.ts
git commit -m "feat(db): add grocery fields and RRA tax type to Product"
```

### Task 10: Batch and stock write-off

**Files:**
- Modify: `prisma/schema.prisma` (`Batch`, `StockWriteOff`, `WriteOffReason` enum; `StockMovement.batchId`, `StockMovement.ebmSyncState`)
- Create: `prisma/migrations/<timestamp>_grocery_models/migration.sql`
- Create: `tests/schema/batch.test.ts`

**Interfaces:**
- Consumes: `Product` from Task 9, `Decimal(12,3)` quantities from Task 7.
- Produces: `Batch { id, productId, lot, receivedAt, bestBefore, orgId }`, `StockWriteOff { id, productId, locationId, batchId?, qty Decimal(12,3), reason WriteOffReason, note?, orgId, createdAt }`, `enum WriteOffReason { EXPIRY DAMAGE THEFT MARKDOWN }`, `enum EbmSyncState { PENDING SENT ACKNOWLEDGED FAILED }`. Phase 3 reads `ebmSyncState`; nothing in this plan writes anything but `PENDING`.

- [ ] **Step 1: Write the failing tests**

```ts
test("a batch carries a best-before date", async () => {
  const b = await prisma.batch.create({
    data: { productId, lot: "L-2026-11", bestBefore: new Date("2026-12-24"), orgId },
  });
  expect(b.bestBefore?.toISOString()).toContain("2026-12-24");
});

test("a write-off records a fractional quantity and a reason", async () => {
  const w = await prisma.stockWriteOff.create({
    data: { productId, locationId, qty: "2.350", reason: "EXPIRY", orgId },
  });
  expect(w.qty.toString()).toBe("2.35");
  expect(w.reason).toBe("EXPIRY");
});

test("a stock movement starts unsynced to EBM", async () => {
  const m = await prisma.stockMovement.create({
    data: { productId, toLocationId: locationId, qty: "1", type: "RECEIPT", orgId },
  });
  expect(m.ebmSyncState).toBe("PENDING");
});

test("deleting a batch does not delete its movements", async () => {
  // batch is a label on stock, not its owner
  const b = await prisma.batch.create({ data: { productId, lot: "L2", orgId } });
  const m = await prisma.stockMovement.create({
    data: { productId, toLocationId: locationId, qty: "1", type: "RECEIPT", orgId, batchId: b.id },
  });
  await prisma.batch.delete({ where: { id: b.id } });
  const still = await prisma.stockMovement.findUnique({ where: { id: m.id } });
  expect(still).not.toBeNull();
  expect(still!.batchId).toBeNull();
});
```

The last test fixes the relation's delete behaviour as `onDelete: SetNull`, which is the decision that test exists to pin.

- [ ] **Step 2: Run to verify failure, implement, migrate**

Run: `npm test -- tests/schema/batch.test.ts` → FAIL
Then: `npx prisma migrate dev --name grocery_models`

`Batch` is optional for every product; whether it becomes mandatory for products with `shelfLifeDays` set is left to the builder per spec §10.

- [ ] **Step 3: Run and commit**

```bash
git add prisma tests/schema/batch.test.ts
git commit -m "feat(db): add Batch, StockWriteOff and EBM sync state"
```

### Task 11: Promotions and price changes

Scope is fixed by Global Constraints: time-boxed price overrides plus line and basket discounts. **No multi-buy, mix-and-match or loyalty conditions.** The schema must not quietly leave room for them, or someone will build them.

**Files:**
- Modify: `prisma/schema.prisma` (`Promotion`, `PriceChange`, `DiscountKind` enum)
- Create: `prisma/migrations/<timestamp>_promotions/migration.sql`
- Create: `tests/schema/promotion.test.ts`

**Interfaces:**
- Consumes: `Product` from Task 9.
- Produces: `Promotion { id, productId, price Decimal(12,2), startsAt, endsAt, orgId }`, `PriceChange { id, productId, oldPrice, newPrice, changedAt, changedById, orgId }`, `enum DiscountKind { PERCENT AMOUNT }`. Task 12's `SaleLine` and `Sale` carry `discountKind` and `discountValue`.

- [ ] **Step 1: Write the failing tests**

```ts
test("a promotion is time-boxed and cannot end before it starts", async () => {
  await expect(prisma.promotion.create({
    data: { productId, price: "900", startsAt: new Date("2026-12-10"),
            endsAt: new Date("2026-12-01"), orgId },
  })).rejects.toThrow();
});

test("overlapping promotions on one product are rejected", async () => {
  await prisma.promotion.create({
    data: { productId, price: "900", startsAt: new Date("2026-12-01"),
            endsAt: new Date("2026-12-10"), orgId },
  });
  await expect(prisma.promotion.create({
    data: { productId, price: "850", startsAt: new Date("2026-12-05"),
            endsAt: new Date("2026-12-15"), orgId },
  })).rejects.toThrow();
});

test("a price change records both sides and who did it", async () => {
  const c = await prisma.priceChange.create({
    data: { productId, oldPrice: "1200", newPrice: "1150", changedById: userId, orgId },
  });
  expect(c.oldPrice.toString()).toBe("1200");
  expect(c.newPrice.toString()).toBe("1150");
});
```

Both rejections need database-level enforcement — a `CHECK (endsAt > startsAt)` constraint and an exclusion constraint on `(productId, tstzrange(startsAt, endsAt))` — added to the generated migration by hand, since Prisma's schema language expresses neither. Overlap rejection is the thing that stops two promotions silently fighting over one price at a till.

- [ ] **Step 2: Run to verify failure, then migrate in two stages**

Run: `npm test -- tests/schema/promotion.test.ts` → FAIL

The two constraints cannot be expressed in Prisma's schema language, so the migration is written in two stages. Editing an already-applied migration does not work — `migrate dev` will not re-run it and will report drift.

1. `npx prisma migrate dev --create-only --name promotions` — generates the SQL without applying it.
2. Hand-edit the generated `migration.sql`, appending:
   - `CREATE EXTENSION IF NOT EXISTS btree_gist;`
   - `ALTER TABLE "Promotion" ADD CONSTRAINT "promotion_dates_ordered" CHECK ("endsAt" > "startsAt");`
   - `ALTER TABLE "Promotion" ADD CONSTRAINT "promotion_no_overlap" EXCLUDE USING gist ("productId" WITH =, tstzrange("startsAt", "endsAt") WITH &&);`
3. `npx prisma migrate dev` — applies it.

- [ ] **Step 3: Run and commit**

```bash
git add prisma tests/schema/promotion.test.ts
git commit -m "feat(db): add time-boxed promotions and price-change history"
```

### Task 12: Till models

The models the Phase 2 Tauri app will consume. Nothing in this plan writes to them beyond tests and the seed — they exist so Phase 2 starts against a settled schema.

**Files:**
- Modify: `prisma/schema.prisma` (`Lane`, `Shift`, `Sale`, `SaleLine`, `Tender`, `PaymentMethod` enum)
- Create: `prisma/migrations/<timestamp>_till_models/migration.sql`
- Create: `tests/schema/till.test.ts`

**Interfaces:**
- Consumes: `Product` (Task 9), `DiscountKind` (Task 11), `Decimal(12,3)` quantities (Task 7), existing `Org`, `User`, `Location`.
- Produces:
  - `Lane { id, code, name, locationId, orgId, mrc String?, bhfId String?, lastInvcNo Int @default(0), createdAt, deletedAt? }` — `@@unique([code, orgId])`
  - `Shift { id, laneId, openedById, openedAt, closedAt?, openingFloat Decimal(12,2), declaredCash Decimal(12,2)?, orgId }`
  - `Sale { id, shiftId, laneId, invcNo Int, rcptNo Int?, totAmt Decimal(12,2), totTaxblAmt, totTaxAmt, buyerTin String?, discountKind DiscountKind?, discountValue Decimal(12,2)?, voidedAt?, orgId, soldAt }` — `@@unique([laneId, invcNo])`
  - `SaleLine { id, saleId, productId, itemSeq Int, qty Decimal(12,3), prc Decimal(12,2), taxTyCd TaxType, discountKind?, discountValue?, orgId }` — `@@unique([saleId, itemSeq])`
  - `Tender { id, saleId, pmtTyCd PaymentMethod, amount Decimal(12,2), providerRef String?, confirmed Boolean @default(false), orgId }`
  - `enum PaymentMethod { CASH CREDIT CASH_CREDIT BANK_CHECK CARD MOBILE_MONEY }`

**Decisions this task locks:**
- **`PaymentMethod` uses names, not RRA's numeric codes,** because a Prisma enum value cannot begin with a digit. The mapping to `01`–`06` lives in Phase 3's `EbmRwandaAdapter`, not in the schema. Order of declaration matches code order so the mapping is positional and obvious.
- **`Lane.lastInvcNo` is a mirror, not the source.** The authoritative gapless sequence lives in the till's own SQLite per spec §7 step 4. This column exists so the back office can show how far a lane has got and detect a gap; nothing allocates from it.
- **`Sale.invcNo` is unique per lane, not per org.** Each lane allocates independently and offline, so a per-org unique would be unsatisfiable.

- [ ] **Step 1: Write the failing tests**

```ts
test("invoice number is unique per lane, not per org", async () => {
  await prisma.sale.create({ data: { ...base, laneId: laneA, invcNo: 1 } });
  await expect(
    prisma.sale.create({ data: { ...base, laneId: laneA, invcNo: 1 } })
  ).rejects.toThrow();
  const b = await prisma.sale.create({ data: { ...base, laneId: laneB, invcNo: 1 } });
  expect(b.invcNo).toBe(1); // two lanes legitimately both have invoice 1
});

test("a sale line holds a weighed quantity and its own tax type", async () => {
  const l = await prisma.saleLine.create({
    data: { saleId, productId, itemSeq: 1, qty: "0.734", prc: "1200", taxTyCd: "B", orgId },
  });
  expect(l.qty.toString()).toBe("0.734");
  expect(l.taxTyCd).toBe("B");
});

test("mobile money is recorded unconfirmed and with no provider reference", async () => {
  const t = await prisma.tender.create({
    data: { saleId, pmtTyCd: "MOBILE_MONEY", amount: "5000", orgId },
  });
  expect(t.confirmed).toBe(false);
  expect(t.providerRef).toBeNull();
});

test("all six RRA payment methods exist", async () => {
  const all = ["CASH","CREDIT","CASH_CREDIT","BANK_CHECK","CARD","MOBILE_MONEY"] as const;
  for (const m of all) {
    const t = await prisma.tender.create({ data: { saleId, pmtTyCd: m, amount: "1", orgId } });
    expect(t.pmtTyCd).toBe(m);
  }
});

test("a sale may carry several tenders that together exceed no constraint", async () => {
  await prisma.tender.create({ data: { saleId, pmtTyCd: "CASH", amount: "3000", orgId } });
  await prisma.tender.create({ data: { saleId, pmtTyCd: "MOBILE_MONEY", amount: "2000", orgId } });
  const ts = await prisma.tender.findMany({ where: { saleId } });
  expect(ts.length).toBeGreaterThanOrEqual(2);
});

test("a sale is voided, never deleted", async () => {
  const s = await prisma.sale.create({ data: { ...base, invcNo: 99 } });
  const v = await prisma.sale.update({ where: { id: s.id }, data: { voidedAt: new Date() } });
  expect(v.voidedAt).not.toBeNull();
});
```

The mixed-tender test records that tender total reconciliation is application logic in Phase 2, not a database constraint — a split payment is built up one tender at a time and would violate any such constraint mid-transaction.

- [ ] **Step 2: Run to verify failure, implement, migrate**

Run: `npm test -- tests/schema/till.test.ts` → FAIL
Then: `npx prisma migrate dev --name till_models`

- [ ] **Step 3: Run and commit**

```bash
git add prisma tests/schema/till.test.ts
git commit -m "feat(db): add Lane, Shift, Sale, SaleLine and Tender"
```

### Task 13: Grocery seed

The spec asks for "a Rwandan supermarket's actual department spread, not 47 widgets". This is what makes Phase 2's lane work demonstrable and what the Phase 2 speed gate is benchmarked against.

**Files:**
- Create: `scripts/seed-grocery.ts`
- Modify: `package.json` (`db:seed:grocery` script)
- Create: `tests/seed/grocery.test.ts`

**Interfaces:**
- Consumes: every model from Tasks 7 and 9–12, and `toQty` from Task 6.
- Produces: no code interface. A seeded org a person can log into.

- [ ] **Step 1: Write the failing test**

```ts
test("the seed produces a plausible supermarket", async () => {
  await seedGrocery(orgId);
  const products = await prisma.product.findMany({ where: { orgId } });
  expect(products.length).toBeGreaterThanOrEqual(120);

  const departments = new Set(products.map((p) => p.department));
  for (const d of ["Fresh Produce", "Butchery", "Dairy", "Bakery", "Dry Goods",
                   "Beverages", "Household", "Personal Care"]) {
    expect(departments).toContain(d);
  }

  const weighed = products.filter((p) => p.isWeighed);
  expect(weighed.length).toBeGreaterThanOrEqual(15);
  expect(weighed.every((p) => p.plu !== null)).toBe(true);

  const exempt = products.filter((p) => p.taxTyCd === "A");
  expect(exempt.length).toBeGreaterThan(0); // staples are exempt in Rwanda

  expect(products.filter((p) => p.barcode !== null).length).toBeGreaterThanOrEqual(100);
});

test("the seed creates lanes and a closed shift with sales", async () => {
  await seedGrocery(orgId);
  expect((await prisma.lane.findMany({ where: { orgId } })).length).toBeGreaterThanOrEqual(3);
  const sales = await prisma.sale.findMany({ where: { orgId }, include: { lines: true, tenders: true } });
  expect(sales.length).toBeGreaterThanOrEqual(50);
  expect(sales.some((s) => s.lines.some((l) => !l.qty.isInteger()))).toBe(true);
  expect(sales.some((s) => s.tenders.length > 1)).toBe(true);
});
```

The two final assertions are the point of the seed: at least one weighed line with a fractional quantity, and at least one split tender. Without those the Phase 2 lane gets built against data that cannot exercise it.

- [ ] **Step 2: Run to verify failure**

Run: `npm test -- tests/seed/grocery.test.ts`
Expected: FAIL, `seedGrocery` is not defined.

- [ ] **Step 3: Implement `scripts/seed-grocery.ts`**

Export `seedGrocery(orgId: string): Promise<void>` and keep the file runnable directly, matching `scripts/seed.ts`. Use real Rwandan supermarket goods and RWF prices across the eight departments. Weighed items (produce, butchery) get a PLU and `isWeighed: true`; packaged goods get an EAN-13 barcode. Staples that are VAT-exempt in Rwanda get `taxTyCd: "A"`; everything else `"B"`. Add three lanes, one closed shift with 50+ sales across them, including fractional weighed lines and at least one cash + mobile-money split.

- [ ] **Step 4: Add the script and run it end to end**

`"db:seed:grocery": "npx tsx scripts/seed-grocery.ts"` in `package.json`.

Run: `npm run db:seed:grocery && npm test`
Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add scripts/seed-grocery.ts package.json tests/seed/grocery.test.ts
git commit -m "feat: seed a Rwandan supermarket with weighed goods, lanes and sales"
```

---

## Self-Review

**Spec coverage.** Phase 0 (§8) → Tasks 1–5. Phase 1 (§8): `Product` grocery fields → Task 9; `Batch`, `StockWriteOff`, `StockMovement.batchId` + EBM sync state → Task 10; `Promotion`, `PriceChange` → Task 11; `Lane`, `Shift`, `Sale`, `SaleLine`, `Tender` with `providerRef`/`confirmed` → Task 12; `qty` → `Decimal` across eight columns, first and alone → Tasks 6–7; grocery seed → Task 13. Global Constraints carry C5b's tax and payment code sets, the §6.3 yellow rule and the §10 promotion scope. Phases 2–5 are explicitly out of scope with reasons.

**Known gaps, deliberate.** `Department` stays a string (Task 9); `Batch` requiredness is left to the builder per §10; tender-total reconciliation is Phase 2 logic, recorded as a test comment in Task 12; `Lane.mrc` is nullable because §9 Q4 has not resolved whether MRC is per org, per lane or per install, and that question must be answered before Phase 3 uses the column.

**Type consistency.** `Prisma.Decimal` is the quantity type from Task 7 onward; `QtyLike`, `toQty`, `addQty`, `subQty`, `isNegative`, `formatQty` are defined once in Task 6 and used under those exact names in Tasks 7, 8 and 13. `TaxType` is defined in Task 9 and reused by `SaleLine` in Task 12. `DiscountKind` is defined in Task 11 and reused by `Sale` and `SaleLine` in Task 12. `readRootTokens` and `contrastRatio` are defined in Task 1 and used in Tasks 1 and 3.

**Review Focus coverage.** #1 `Decimal + number` → Task 6 steps 1 and 4. #2 precision beyond scale → Task 7 step 1, with Task 8 step 1 rejecting it earlier at the boundary. #3 decimal CSV import → Task 8 step 1. #4 missing tax code → Task 9 step 1, resolved by defaulting to `B` with the reasoning recorded. #5 weight-embedded barcodes → Task 9 step 1.

**Proportion.** Thirteen tasks against a 455-line spec. Code blocks are tests and exact values; implementation steps carry signatures and the specific decisions, not bodies. The two places a body is pinned — the Shelf Edge palette and the vocabulary table — are exact copy and exact hex values the spec fixes, which an implementer cannot derive.
