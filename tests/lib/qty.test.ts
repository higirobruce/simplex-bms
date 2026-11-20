import { expect, test } from "vitest";
import { Prisma } from "@prisma/client";
import { addQty, subQty, isNegative, formatQty } from "@/lib/qty";

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

test("isNegative treats negative zero as not negative", () => {
  expect(isNegative(new Prisma.Decimal("-0"))).toBe(false);
});

test("addQty with no arguments returns 0", () => {
  expect(addQty().toString()).toBe("0");
});

test("formatQty distinguishes weighed from each", () => {
  expect(formatQty(D("0.734"), true)).toBe("0.734");
  expect(formatQty(D("3"), false)).toBe("3");
  expect(formatQty(D("3.000"), false)).toBe("3");
});
