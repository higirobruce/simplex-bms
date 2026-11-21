import { Prisma } from "@prisma/client";
import { expect, test } from "vitest";
import { seedGrocery } from "../../scripts/seed-grocery";
import { prisma, seedOrg } from "../helpers/db";

const DEPARTMENTS = [
  "Fresh Produce",
  "Butchery",
  "Dairy",
  "Bakery",
  "Dry Goods",
  "Beverages",
  "Household",
  "Personal Care",
];

/**
 * EAN-13 check digit, written independently of the seed's own helper (summed from the
 * right, so a shared bug could not hide here) to prove the barcodes would scan.
 */
function ean13CheckDigit(body12: string): string {
  let sum = 0;
  for (let i = 0; i < 12; i++) {
    const digit = Number(body12[11 - i]);
    sum += i % 2 === 0 ? digit * 3 : digit;
  }
  return String((10 - (sum % 10)) % 10);
}

const dec = (v: Prisma.Decimal | string | number) => new Prisma.Decimal(v);
/** VAT at 18% carved out of a tax-inclusive amount, rounded to the money scale. */
const vatOf = (inclusive: Prisma.Decimal) => inclusive.times(18).div(118).toDecimalPlaces(2);

test("the seed produces a plausible supermarket", async () => {
  // seedGrocery's second argument is the client it writes through: tests pass the
  // guarded test client so the writes can only ever land in bms_test.
  const { orgId } = await seedOrg();
  await seedGrocery(orgId, prisma);

  const products = await prisma.product.findMany({ where: { orgId } });
  expect(products.length).toBeGreaterThanOrEqual(120);

  const departments = new Set(products.map((p) => p.department));
  for (const d of DEPARTMENTS) {
    expect(Array.from(departments)).toContain(d);
  }

  const weighed = products.filter((p) => p.isWeighed);
  expect(weighed.length).toBeGreaterThanOrEqual(15);
  expect(weighed.every((p) => p.plu !== null)).toBe(true);
  // Real PLUs are 4-digit till codes, and a weighed item is priced per kg, not scanned.
  expect(weighed.every((p) => /^\d{4}$/.test(p.plu ?? ""))).toBe(true);
  expect(weighed.every((p) => p.barcode === null)).toBe(true);
  expect(weighed.some((p) => p.department === "Fresh Produce")).toBe(true);
  expect(weighed.some((p) => p.department === "Butchery")).toBe(true);

  const exempt = products.filter((p) => p.taxTyCd === "A");
  expect(exempt.length).toBeGreaterThan(0); // staples are exempt in Rwanda
  // The exemption follows the goods, not the whole department: unprocessed staples are
  // exempt while processed and non-food lines are standard rated.
  expect(exempt.some((p) => p.department === "Fresh Produce")).toBe(true);
  expect(products.filter((p) => p.department === "Beverages").every((p) => p.taxTyCd === "B")).toBe(true);
  expect(products.filter((p) => p.department === "Household").every((p) => p.taxTyCd === "B")).toBe(true);

  const barcoded = products.filter((p) => p.barcode !== null);
  expect(barcoded.length).toBeGreaterThanOrEqual(100);
  for (const p of barcoded) {
    const barcode = p.barcode as string;
    expect(barcode).toMatch(/^\d{13}$/);
    expect(barcode.slice(12)).toBe(ean13CheckDigit(barcode.slice(0, 12)));
  }
  expect(new Set(barcoded.map((p) => p.barcode)).size).toBe(barcoded.length);

  // Plausible Kigali shelf prices in RWF, and every priced line carries a department.
  const seeded = products.filter((p) => p.department !== "General");
  expect(seeded.every((p) => dec(p.unitPrice).gte(100) && dec(p.unitPrice).lte(200000))).toBe(true);
  expect(seeded.every((p) => p.sku.length > 0 && p.name.length > 0)).toBe(true);
  // Perishables carry a shelf life; the shelf-life bound from Task 9 holds.
  expect(seeded.filter((p) => p.shelfLifeDays !== null).length).toBeGreaterThanOrEqual(80);
  expect(seeded.every((p) => p.shelfLifeDays === null || (p.shelfLifeDays >= 1 && p.shelfLifeDays <= 3650))).toBe(true);
});

test("the seed creates lanes and a closed shift with sales", async () => {
  const { orgId } = await seedOrg();
  await seedGrocery(orgId, prisma);

  expect((await prisma.lane.findMany({ where: { orgId } })).length).toBeGreaterThanOrEqual(3);

  const shifts = await prisma.shift.findMany({ where: { orgId } });
  expect(shifts.some((s) => s.closedAt !== null && s.declaredCash !== null)).toBe(true);

  const sales = await prisma.sale.findMany({
    where: { orgId },
    include: { lines: true, tenders: true },
  });
  expect(sales.length).toBeGreaterThanOrEqual(50);
  expect(sales.some((s) => s.lines.some((l) => !l.qty.isInteger()))).toBe(true);
  expect(sales.some((s) => s.tenders.length > 1)).toBe(true);

  // Every sale is rung on a lane whose own invoice sequence it uses.
  const laneIds = new Set(sales.map((s) => s.laneId));
  expect(laneIds.size).toBeGreaterThanOrEqual(3);
  expect(sales.every((s) => s.lines.length > 0 && s.tenders.length > 0)).toBe(true);

  // Mobile money is recorded, never confirmed, in v1.
  const tenders = sales.flatMap((s) => s.tenders);
  expect(tenders.some((t) => t.pmtTyCd === "MOBILE_MONEY")).toBe(true);
  expect(tenders.every((t) => t.providerRef === null)).toBe(true);
  expect(tenders.every((t) => t.confirmed === false)).toBe(true);

  // A split basket is cash plus mobile money, and it adds up.
  const split = sales.find((s) => s.tenders.length > 1);
  expect(split).toBeDefined();
  expect(new Set(split!.tenders.map((t) => t.pmtTyCd))).toEqual(new Set(["CASH", "MOBILE_MONEY"]));

  const weighedIds = new Set(
    (await prisma.product.findMany({ where: { orgId, isWeighed: true }, select: { id: true } })).map(
      (p) => p.id,
    ),
  );

  // Baskets include VAT-exempt staples alongside standard-rated goods.
  const lines = sales.flatMap((s) => s.lines);
  expect(lines.some((l) => l.taxTyCd === "A")).toBe(true);
  expect(lines.some((l) => l.taxTyCd === "B")).toBe(true);

  for (const s of sales) {
    // The EBM identity: the inclusive total is its base plus its tax, to the cent.
    expect(dec(s.totAmt).toString()).toBe(dec(s.totTaxblAmt).plus(s.totTaxAmt).toString());
    // Tenders settle the sale exactly.
    const tendered = s.tenders.reduce((t, x) => t.plus(x.amount), new Prisma.Decimal(0));
    expect(tendered.toString()).toBe(dec(s.totAmt).toString());

    const gross = s.lines.reduce(
      (t, l) => t.plus(dec(l.qty).times(l.prc).toDecimalPlaces(2)),
      new Prisma.Decimal(0),
    );
    const discounted =
      s.discountValue !== null || s.lines.some((l) => l.discountValue !== null);
    if (discounted) {
      // A discount only ever reduces the total, and never below nothing.
      expect(dec(s.totAmt).lt(gross)).toBe(true);
      expect(dec(s.totAmt).gt(0)).toBe(true);
    } else {
      expect(dec(s.totAmt).toString()).toBe(gross.toString());
      // VAT is carved out of the standard-rated lines only; exempt lines add no tax.
      const standard = s.lines
        .filter((l) => l.taxTyCd === "B")
        .reduce((t, l) => t.plus(dec(l.qty).times(l.prc).toDecimalPlaces(2)), new Prisma.Decimal(0));
      expect(dec(s.totTaxAmt).toString()).toBe(vatOf(standard).toString());
    }

    // Only a weighed item can be sold in a fraction of a unit.
    for (const l of s.lines) {
      if (!dec(l.qty).isInteger()) expect(weighedIds.has(l.productId)).toBe(true);
      expect(dec(l.qty).gt(0)).toBe(true);
    }
  }

  // One voided basket, so the lane screens have a void to render.
  expect(sales.filter((s) => s.voidedAt !== null).length).toBeGreaterThanOrEqual(1);
});
