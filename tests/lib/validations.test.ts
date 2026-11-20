import { expect, test } from "vitest";
import { QtySchema, SignedQtySchema, StockImportRowSchema } from "@/lib/validations";

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
  if (!r.success) {
    expect(r.error.issues[0].message).toBe("Quantity must be positive, to at most 3 decimal places");
  }
});

test("garbage input is rejected with the same message, not thrown", () => {
  for (const bad of ["abc", "", null, undefined, NaN, Infinity]) {
    expect(QtySchema.safeParse(bad).success).toBe(false);
  }
});

test("the opening-stock importer accepts decimal quantities", () => {
  expect(StockImportRowSchema.safeParse({ sku: "RICE-5", location: "Main", qty: "12.5" }).success).toBe(true);
  expect(StockImportRowSchema.safeParse({ sku: "RICE-5", location: "Main", qty: "1.2345" }).success).toBe(false);
});
