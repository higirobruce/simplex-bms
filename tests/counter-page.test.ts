import { describe, expect, test } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CounterView, ShiftSales } from "@/components/counter-view";
import { absMoney, groupMoney, shiftDay, varianceKind, type CounterResponse, type CounterShift } from "@/lib/counter-view";

// No React testing library in this project: components are rendered to static
// HTML with react-dom/server and asserted on the markup. Times pinned to UTC.
const TZ = "UTC";
const NOW = new Date("2026-11-22T09:00:00Z");

function shift(over: Partial<CounterShift> = {}): CounterShift {
  return {
    id: "s1",
    lane: { id: "l1", code: "L01", name: "Lane 02" },
    cashier: { id: "u1", name: "M. Uwase" },
    openedAt: "2026-11-21T06:00:00Z",
    closedAt: "2026-11-21T14:10:00Z",
    lastReachedAt: "2026-11-21T14:11:00Z",
    state: "complete",
    openingFloat: "10000.00",
    saleCount: 142,
    tillSaleCount: 142,
    takings: "356500.00",
    tillGross: "356500.00",
    methods: [{ method: "CASH", tendered: "218000.00" }, { method: "MOBILE_MONEY", tendered: "84000.00" }],
    changeGiven: "6500.00",
    variance: { expected: "231500.00", declared: "229000.00", diff: "-2500.00" },
    ...over,
  };
}
const view = (shifts: CounterShift[], extra: Partial<CounterResponse> = {}) =>
  renderToStaticMarkup(
    createElement(CounterView, {
      data: { day: "2026-11-21", timezone: TZ, shifts, total: "356500.00", totalQualified: true, ...extra },
      now: NOW,
      tz: (extra.timezone ?? TZ),
    }),
  );

/** The one card for a state; fails loudly if it is absent or duplicated. */
function card(h: string, state: string, id?: string): string {
  const re = new RegExp(`<article data-state="${state}"[^>]*>[\\s\\S]*?</article>`, "g");
  const all = h.match(re) ?? [];
  expect(all.length, `cards for ${state}`).toBeGreaterThan(0);
  return id ? (all.find((c) => c.includes(id)) ?? "") : (all[0] ?? "");
}

// A real, populated variance. A withheld state must NOT print any part of it.
const POPULATED = { expected: "123456.00", declared: "119135.00", diff: "-4321.00" };
const POPULATED_TEXT = ["4,321", "123,456", "119,135", "Short", "Over", "Balanced", "expected"];
function expectNoVariance(c: string) {
  for (const t of POPULATED_TEXT) expect(c, `card leaked "${t}"`).not.toContain(t);
}

describe("counter page", () => {
  test("a short shows the shortfall, the expected drawer and the counted cash", () => {
    const h = view([shift()]);
    expect(h).toContain("Short 2,500");
    expect(h).toContain("expected 231,500 in the drawer, counted 229,000");
    expect(h).toContain("M. Uwase");
    expect(h).toMatch(/<span class="[^"]*text-danger[^"]*">Short 2,500<\/span>/);
  });

  test("an over reads as over, never as a negative short", () => {
    const h = view([shift({ variance: { expected: "231500.00", declared: "234000.00", diff: "2500.00" } })]);
    expect(h).toContain("Over 2,500");
    expect(h).not.toMatch(/Short|-2,500|−2,500/);
    expect(h).toMatch(/<span class="[^"]*text-info[^"]*">Over 2,500<\/span>/);
    expect(h).not.toMatch(/<span class="[^"]*text-danger[^"]*">Over/);
  });

  test("a balanced drawer says balanced", () => {
    const h = view([shift({ variance: { expected: "5.00", declared: "5.00", diff: "0.00" } })]);
    expect(h).toContain("Balanced");
    expect(h).not.toMatch(/Short|Over/);
  });

  test("a stale open shift is flagged and says when the lane was last reachable", () => {
    const h = view(
      [shift({ state: "stale", closedAt: null, openedAt: "2026-11-21T07:30:00Z", lastReachedAt: "2026-11-21T20:51:00Z", variance: null })],
    );
    expect(h).toContain("Stuck — check this lane");
    expect(h).toContain("still open from 07:30 yesterday");
    expect(h).toContain("not reachable since 20:51 yesterday");
    expect(h).toContain('role="alert"');
    expect(h).not.toContain(">Trading<");
    expect(h).not.toMatch(/Short|Over|Balanced/);
  });

  test("a stale shift whose lane was never reached says so", () => {
    const h = view([shift({ state: "stale", closedAt: null, lastReachedAt: null, variance: null })]);
    expect(h).toContain("never been reachable");
  });

  test("an open shift is trading, with no variance and no alarm", () => {
    const h = view([shift({ state: "open", closedAt: null, openedAt: "2026-11-22T06:02:00Z", lastReachedAt: "2026-11-22T08:55:00Z", variance: null })]);
    expect(h).toContain(">Trading<");
    expect(h).toContain("lane reachable 08:55");
    expect(h).not.toContain('role="alert"');
    expect(h).not.toMatch(/Short|Over|Balanced|Variance/);
  });

  test("a short_count shift shows NO variance number and names the gap", () => {
    const h = view([shift({ state: "short_count", tillSaleCount: 145, saleCount: 142, variance: POPULATED })]);
    const c = card(h, "short_count");
    expect(c).toContain("Variance withheld");
    expect(c).toContain("reports 145 sales, we hold 142");
    expect(c).toContain("3 not here yet");
    expectNoVariance(c);
  });

  test("an unknown shift shows its own takings and no variance", () => {
    const h = view([shift({ state: "unknown", tillSaleCount: null, tillGross: null, takings: "412350.00", variance: POPULATED })]);
    const c = card(h, "unknown");
    expect(c).toContain("412,350");
    expect(c).toContain("Variance not available");
    expectNoVariance(c);
  });

  test("a mismatch is a data fault, its row louder than lateness, with no variance", () => {
    const h = view([shift({ state: "mismatch", variance: POPULATED })]);
    const c = card(h, "mismatch");
    expect(c).toContain("data fault");
    // the verdict ROW wrapper is on the danger ground, not merely the tag
    expect(c).toMatch(/<div class="[^"]*border-t[^"]*bg-danger-soft[^"]*">/);
    expectNoVariance(c);
  });

  test("a withheld stale shift shows no variance either", () => {
    const h = view([shift({ state: "stale", closedAt: null, variance: POPULATED })]);
    expectNoVariance(card(h, "stale"));
  });

  test("a complete shift's row is not on the loud ground", () => {
    expect(card(view([shift()]), "complete")).not.toMatch(/border-t[^"]*bg-danger-soft/);
  });

  test("several shifts at once: each card carries only its own variance text", () => {
    const h = view([
      shift({ id: "a", variance: { expected: "231500.00", declared: "229000.00", diff: "-2500.00" } }),
      shift({ id: "b", cashier: { id: "u2", name: "P. Habimana" }, variance: { expected: "158800.00", declared: "158800.00", diff: "0.00" } }),
      shift({ id: "c", cashier: { id: "u3", name: "J. Mukamana" }, state: "short_count", tillSaleCount: 145, saleCount: 142, variance: POPULATED }),
      shift({ id: "d", cashier: { id: "u4", name: "K. Nsabimana" }, state: "unknown", tillSaleCount: null, tillGross: null, variance: POPULATED }),
      shift({ id: "e", cashier: { id: "u5", name: "A. Ingabire" }, state: "mismatch", variance: POPULATED }),
      shift({ id: "f", cashier: { id: "u6", name: "C. Mugisha" }, state: "stale", closedAt: null, variance: POPULATED }),
    ]);
    const complete = h.match(/<article data-state="complete"[\s\S]*?<\/article>/g) ?? [];
    expect(complete).toHaveLength(2);
    const short = complete.find((c) => c.includes("M. Uwase"))!;
    const balanced = complete.find((c) => c.includes("P. Habimana"))!;
    expect(short).toContain("Short 2,500");
    expect(short).not.toContain("Balanced");
    expect(balanced).toContain("Balanced");
    expect(balanced).not.toMatch(/Short|Over|2,500/);
    for (const st of ["short_count", "unknown", "mismatch", "stale"]) expectNoVariance(card(h, st));
  });

  test("a shift's displayed time follows the org's zone, not the runtime's", () => {
    const s = shift({ openedAt: "2026-11-21T22:30:00Z", closedAt: "2026-11-22T06:15:00Z" });
    const kigali = view([s], { timezone: "Africa/Kigali" });
    expect(card(kigali, "complete")).toContain("00:30 → 08:15");
    const la = view([s], { timezone: "America/Los_Angeles" });
    expect(card(la, "complete")).toContain("14:30 → 22:15");
    expect(card(view([s], { timezone: "UTC" }), "complete")).toContain("22:30 → 06:15");
  });

  test("the six states do not render alike", () => {
    const states = ["open", "stale", "complete", "short_count", "unknown", "mismatch"] as const;
    const out = new Set(states.map((state) => view([shift({ state, closedAt: state === "open" || state === "stale" ? null : "2026-11-21T14:10:00Z", variance: null })])));
    expect(out.size).toBe(6);
  });

  test("the shop total sits below the shifts and says when it is incomplete", () => {
    const h = view([shift()], { totalQualified: false, total: "882800.00" });
    expect(h.indexOf("Short 2,500")).toBeLessThan(h.indexOf("882,800"));
    expect(h).toContain("Incomplete");
    expect(view([shift()])).not.toContain("Incomplete");
  });

  test("a complete shift without a cash count shows no variance", () => {
    const h = view([shift({ variance: null })]);
    expect(h).toContain("No cash count recorded");
    expect(h).not.toMatch(/Short \d|Over \d|Balanced/);
  });

  test("nothing on the page is an input or a button that mutates", () => {
    const h = view([
      shift(),
      shift({ id: "s2", state: "stale", closedAt: null, variance: null }),
      shift({ id: "s3", state: "mismatch", variance: null }),
    ]);
    expect(h).not.toMatch(/<(input|button|select|textarea|form)\b/i);
  });
});

describe("counter-view helpers", () => {
  test("money is re-grouped as text, never parsed", () => {
    expect(groupMoney("1234567.50")).toBe("1,234,567.50");
    expect(groupMoney("356500.00")).toBe("356,500");
    expect(groupMoney("12345678901234567890.10")).toBe("12,345,678,901,234,567,890.10");
    expect(absMoney("-2500.00")).toBe("2,500");
  });
  test("variance kind", () => {
    expect(varianceKind("-0.01")).toBe("short");
    expect(varianceKind("0.01")).toBe("over");
    expect(varianceKind("0.00")).toBe("balanced");
    expect(varianceKind("-0.00")).toBe("balanced");
  });
  test("day navigation crosses month and year", () => {
    expect(shiftDay("2026-03-01", -1)).toBe("2026-02-28");
    expect(shiftDay("2026-12-31", 1)).toBe("2027-01-01");
  });
});

describe("ShiftSales (expanded row)", () => {
  const data = {
    shiftId: "s1",
    record: { kind: "back-office" as const, fiscalCopy: false as const, note: "Back-office record, not the fiscal receipt." },
    sales: [{
      id: "x1", invcNo: 7, rcptNo: 7, soldAt: "2026-11-21T09:30:00Z", voidedAt: null,
      totAmt: "1000.00", totTaxblAmt: "847.46", totTaxAmt: "152.54", buyerTin: null,
      lines: [
        { itemSeq: 1, name: "Rice 1kg", qty: "3.333", prc: "100.00", taxTyCd: "B", splyAmt: "334.00", taxblAmt: "283.05" },
        { itemSeq: 2, name: "Old line", qty: "1.000", prc: "666.00", taxTyCd: "B", splyAmt: null, taxblAmt: null },
      ],
      tenders: [{ seq: 1, method: "CASH", amount: "1000.00" }],
    }],
  };
  const html = () => renderToStaticMarkup(createElement(ShiftSales, { data, tz: TZ }));

  test("renders the stored line amount, not qty x price", () => {
    expect(html()).toContain(">334<");
    expect(html()).not.toContain(">333<");
  });
  test("a line with no stored amount shows a dash, never a derived figure", () => {
    expect(html()).not.toContain(">667<");
    expect(html()).toContain(">—<");
  });
  test("carries the back-office flag and has no inputs or mutating controls", () => {
    expect(html()).toContain("not the fiscal receipt");
    expect(html()).not.toMatch(/<(input|button|form|select|textarea)/);
  });
  test("the expander is a plain toggle only when a slug is supplied", () => {
    const base = { day: "2026-11-21", timezone: TZ, shifts: [shift()], total: "1", totalQualified: true };
    const withSlug = renderToStaticMarkup(createElement(CounterView, { data: base, now: NOW, tz: TZ, slug: "shop" }));
    expect(withSlug).toContain('aria-expanded="false"');
    expect(withSlug).not.toMatch(/<(input|form|select|textarea)/);
  });
});
