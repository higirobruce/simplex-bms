import { describe, expect, test } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CounterView } from "@/components/counter-view";
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
      data: { day: "2026-11-21", shifts, total: "356500.00", totalQualified: true, ...extra },
      now: NOW,
      tz: TZ,
    }),
  );

describe("counter page", () => {
  test("a short shows the shortfall, the expected drawer and the counted cash", () => {
    const h = view([shift()]);
    expect(h).toContain("Short 2,500");
    expect(h).toContain("expected 231,500 in the drawer, counted 229,000");
    expect(h).toContain("M. Uwase");
    expect(h).toContain("text-danger");
  });

  test("an over reads as over, never as a negative short", () => {
    const h = view([shift({ variance: { expected: "231500.00", declared: "234000.00", diff: "2500.00" } })]);
    expect(h).toContain("Over 2,500");
    expect(h).not.toMatch(/Short|-2,500|−2,500/);
    expect(h).toContain("text-info");
    expect(h).not.toContain("font-semibold text-danger");
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
    const h = view([shift({ state: "short_count", tillSaleCount: 145, saleCount: 142, variance: null })]);
    expect(h).toContain("Variance withheld");
    expect(h).toContain("reports 145 sales, we hold 142");
    expect(h).toContain("3 not here yet");
    expect(h).not.toMatch(/Short \d|Over \d|Balanced|expected/);
  });

  test("an unknown shift shows takings and no variance", () => {
    const h = view([shift({ state: "unknown", tillSaleCount: null, tillGross: null, variance: null })]);
    expect(h).toContain("356,500");
    expect(h).toContain("Variance not available");
    expect(h).not.toMatch(/Short \d|Over \d|Balanced|expected/);
  });

  test("a mismatch is a data fault, louder than lateness, with no variance", () => {
    const h = view([shift({ state: "mismatch", variance: null })]);
    expect(h).toContain("data fault");
    expect(h).toContain("bg-danger-soft");
    expect(h).not.toMatch(/Short \d|Over \d|Balanced|expected/);
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
