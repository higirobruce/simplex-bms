import { describe, expect, test, vi } from "vitest";
import { dayBounds, startOfMonthIn } from "@/lib/day";

const KIGALI = "Africa/Kigali";

describe("day bounds in the org's timezone", () => {
  test("a Kigali day is bounded at 22:00 UTC the previous evening", () => {
    const { start, end } = dayBounds(new Date("2026-11-21T10:00:00Z"), KIGALI);
    expect(start.toISOString()).toBe("2026-11-20T22:00:00.000Z");
    expect(end.toISOString()).toBe("2026-11-21T22:00:00.000Z");
  });

  test("a sale at 01:30 Kigali belongs to that Kigali day, not the UTC one", () => {
    const sale = new Date("2026-11-20T23:30:00Z"); // Sat 01:30 Kigali, Fri in UTC
    const { start, end } = dayBounds(sale, KIGALI);
    expect(start.toISOString()).toBe("2026-11-20T22:00:00.000Z");
    expect(sale >= start && sale < end).toBe(true);
    const utc = dayBounds(sale, "UTC");
    expect(utc.start.toISOString()).toBe("2026-11-20T00:00:00.000Z");
  });

  test("start is inclusive and end is exclusive", () => {
    const { start, end } = dayBounds(new Date("2026-11-21T10:00:00Z"), KIGALI);
    expect(dayBounds(start, KIGALI).start.getTime()).toBe(start.getTime());
    expect(dayBounds(end, KIGALI).start.getTime()).toBe(end.getTime());
    expect(dayBounds(new Date(end.getTime() - 1), KIGALI).end.getTime()).toBe(end.getTime());
  });

  test("the month boundary is the first of the month in the zone, not the server's", () => {
    // 23:30 UTC on 30 Nov is already 1 Dec in Kigali.
    expect(startOfMonthIn(new Date("2026-11-30T23:30:00Z"), KIGALI).toISOString())
      .toBe("2026-11-30T22:00:00.000Z");
    // Mid-month: Kigali start-of-month is 22:00 UTC on the 31st of the prior month.
    expect(startOfMonthIn(new Date("2026-11-15T12:00:00Z"), KIGALI).toISOString())
      .toBe("2026-10-31T22:00:00.000Z");
    expect(startOfMonthIn(new Date("2026-11-15T12:00:00Z"), "UTC").toISOString())
      .toBe("2026-11-01T00:00:00.000Z");
  });

  test("an unknown zone falls back to UTC rather than throwing", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const d = new Date("2026-11-21T10:00:00Z");
    for (const tz of ["Not/AZone", "", "garbage 123"]) {
      const { start, end } = dayBounds(d, tz);
      expect(start.toISOString()).toBe("2026-11-21T00:00:00.000Z");
      expect(end.toISOString()).toBe("2026-11-22T00:00:00.000Z");
      expect(startOfMonthIn(d, tz).toISOString()).toBe("2026-11-01T00:00:00.000Z");
    }
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  test("an invalid date does not throw", () => {
    expect(() => dayBounds(new Date(NaN), KIGALI)).not.toThrow();
  });

  test("honours DST for zones that have it (no fixed offset)", () => {
    // New York: UTC-5 in winter, UTC-4 in summer.
    expect(dayBounds(new Date("2026-01-15T12:00:00Z"), "America/New_York").start.toISOString())
      .toBe("2026-01-15T05:00:00.000Z");
    expect(dayBounds(new Date("2026-07-15T12:00:00Z"), "America/New_York").start.toISOString())
      .toBe("2026-07-15T04:00:00.000Z");
    // Spring-forward day (2026-03-08) is 23 hours long.
    const b = dayBounds(new Date("2026-03-08T18:00:00Z"), "America/New_York");
    expect((b.end.getTime() - b.start.getTime()) / 3600000).toBe(23);
  });

  test("handles half-hour offsets", () => {
    expect(dayBounds(new Date("2026-11-21T10:00:00Z"), "Asia/Kolkata").start.toISOString())
      .toBe("2026-11-20T18:30:00.000Z");
  });
});
