import { describe, expect, test } from "vitest";
import { instantOnDay, shiftState, splitTakings, variance } from "@/lib/counter";

const now = new Date("2026-11-22T10:00:00.000Z");
const ago = (min: number) => new Date(now.getTime() - min * 60_000);
const dayStart = new Date("2026-11-21T22:00:00.000Z");
const open = { closedAt: null, saleCount: null, gross: null, heldCount: 0, heldGross: "0.00", now, openedAt: ago(120), dayStart };
const closed = { ...open, closedAt: ago(5), lastSeenAt: ago(1) };

describe("shiftState", () => {
  test("an open shift seen two minutes ago is open, not stale", () => {
    expect(shiftState({ ...open, lastSeenAt: ago(2) })).toBe("open");
  });
  test("an open shift not seen for 21 minutes is stale", () => {
    expect(shiftState({ ...open, lastSeenAt: ago(21) })).toBe("stale");
  });
  test("an open shift carried over from yesterday is stale however recently seen", () => {
    expect(shiftState({ ...open, lastSeenAt: ago(0), openedAt: new Date("2026-11-21T10:00:00Z") })).toBe("stale");
  });
  test("a lane that has never reported does not crash and is not called reachable", () => {
    expect(shiftState({ ...open, lastSeenAt: null })).toBe("stale");
  });
  test("counts and gross matching is complete", () => {
    expect(shiftState({ ...closed, saleCount: 2, gross: "300.00", heldCount: 2, heldGross: "300" })).toBe("complete");
  });
  test("the till reporting more sales than we hold is short_count", () => {
    expect(shiftState({ ...closed, saleCount: 3, gross: "300.00", heldCount: 2, heldGross: "200.00" })).toBe("short_count");
  });
  test("counts matching while gross differs is mismatch, never complete", () => {
    expect(shiftState({ ...closed, saleCount: 2, gross: "300.00", heldCount: 2, heldGross: "299.00" })).toBe("mismatch");
  });
  test("we hold more sales than the till reports is mismatch", () => {
    expect(shiftState({ ...closed, saleCount: 1, gross: "100.00", heldCount: 2, heldGross: "100.00" })).toBe("mismatch");
  });
  test("neither field present is unknown", () => {
    expect(shiftState({ ...closed, saleCount: null, gross: null })).toBe("unknown");
  });
  test("a till that reported zero sales is complete, not unknown", () => {
    expect(shiftState({ ...closed, saleCount: 0, gross: "0.00", heldCount: 0, heldGross: "0.00" })).toBe("complete");
  });
});

describe("variance", () => {
  test("expected drawer is float plus cash tendered minus change given", () => {
    expect(variance({ openingFloat: "20000.00", cashTendered: "250000.00", changeGiven: "38500.00", declaredCash: null }).expected).toBe("231500.00");
  });
  test("a shift that sold nothing expects exactly its float", () => {
    expect(variance({ openingFloat: "20000.00", cashTendered: "0.00", changeGiven: "0.00", declaredCash: "20000.00" })).toEqual({
      expected: "20000.00", declared: "20000.00", diff: "0.00",
    });
  });
  test("declared above expected yields a positive diff", () => {
    expect(variance({ openingFloat: "100.00", cashTendered: "0.00", changeGiven: "0.00", declaredCash: "150.50" }).diff).toBe("50.50");
  });
  test("declared below expected is a negative diff (short)", () => {
    expect(variance({ openingFloat: "231500.00", cashTendered: "0", changeGiven: "0", declaredCash: "229000.00" }).diff).toBe("-2500.00");
  });
  test("no declaredCash yields a null diff rather than a zero", () => {
    const v = variance({ openingFloat: "100.00", cashTendered: "0.00", changeGiven: "0.00", declaredCash: null });
    expect(v.diff).toBeNull();
    expect(v.declared).toBeNull();
  });
  test("decimals stay exact", () => {
    expect(variance({ openingFloat: "0.10", cashTendered: "0.20", changeGiven: "0.00", declaredCash: "0.30" }).diff).toBe("0.00");
  });
});

describe("splitTakings", () => {
  test("methods minus change equals takings", () => {
    const s = splitTakings([
      { totAmt: "700.00", tenders: [{ pmtTyCd: "CASH", amount: "1000.00" }] },
      { totAmt: "500.00", tenders: [{ pmtTyCd: "CASH", amount: "200.00" }, { pmtTyCd: "CARD", amount: "300.00" }] },
    ]);
    expect(s.takings).toBe("1200.00");
    expect(s.changeGiven).toBe("300.00");
    expect(s.reconciles).toBe(true);
  });
  test("change is capped by the cash taken", () => {
    const s = splitTakings([{ totAmt: "100.00", tenders: [{ pmtTyCd: "CARD", amount: "150.00" }, { pmtTyCd: "CASH", amount: "20.00" }] }]);
    expect(s.changeGiven).toBe("20.00");
    expect(s.reconciles).toBe(false);
  });
});

describe("instantOnDay", () => {
  test("rejects non-dates and finds the day in far zones", () => {
    expect(instantOnDay("2026-02-30", "Africa/Kigali")).toBeNull();
    expect(instantOnDay("nope", "Africa/Kigali")).toBeNull();
    expect(instantOnDay("2026-11-22", "Pacific/Kiritimati")).not.toBeNull();
    expect(instantOnDay("2026-11-22", "Pacific/Pago_Pago")).not.toBeNull();
  });
});
