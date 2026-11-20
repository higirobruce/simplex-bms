import { expect, test } from "vitest";
import { isFullyDelivered, remainingQty, sumQty } from "@/lib/qty-display";

test("string comparison would call 10 less than 9; the helper compares numerically", () => {
  expect("10" >= "9").toBe(false); // the lexicographic trap, documented
  expect(isFullyDelivered("10", "9")).toBe(true);
  expect(isFullyDelivered("9", "10")).toBe(false);
});

test("exactly delivered counts as fully delivered; nothing delivered does not", () => {
  expect(isFullyDelivered("2.500", "2.5")).toBe(true);
  expect(isFullyDelivered("0", "5")).toBe(false);
});

test("remainingQty has no float noise", () => {
  expect(0.3 - 0.1).not.toBe(0.2);
  expect(remainingQty("0.3", "0.1")).toBe(0.2);
  expect(remainingQty("10", "9.999")).toBe(0.001);
  expect(remainingQty("5", "5")).toBe(0);
});

test("sumQty rounds to three places so a threshold comparison is exact", () => {
  expect(0.1 + 0.2 <= 0.3).toBe(false); // the trap
  expect(sumQty(["0.1", "0.2"]) <= 0.3).toBe(true);
  expect(sumQty([])).toBe(0);
});

import { parseQtyInput } from "@/lib/qty-display";

test("parseQtyInput keeps fractions instead of truncating them", () => {
  expect(parseInt("0.734")).toBe(0); // the bug it replaces
  expect(parseQtyInput("0.734")).toBe(0.734);
  expect(parseQtyInput("2.5")).toBe(2.5);
  expect(parseQtyInput("12.75")).toBe(12.75);
  expect(parseQtyInput("-2.5")).toBe(-2.5);
  expect(parseQtyInput(".5")).toBe(0.5);
});

test("parseQtyInput returns null for empty or non-numeric text, never a fallback", () => {
  expect(parseQtyInput("")).toBeNull();
  expect(parseQtyInput("  ")).toBeNull();
  expect(parseQtyInput("abc")).toBeNull();
  expect(parseQtyInput("1e3")).toBeNull();
  expect(parseQtyInput("1.2.3")).toBeNull();
});
