import { Prisma } from "@prisma/client";
import { expect, test } from "vitest";
import { exVatPrice, seedGrocery } from "../../scripts/seed-grocery";
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

/** What a discountKind plus discountValue takes off an amount, at line or basket level. */
function discountOff(
  amount: Prisma.Decimal,
  kind: "PERCENT" | "AMOUNT" | null,
  value: Prisma.Decimal | null,
): Prisma.Decimal {
  if (kind === null || value === null) return new Prisma.Decimal(0);
  if (kind === "PERCENT") return amount.times(value).div(100).toDecimalPlaces(2);
  return Prisma.Decimal.min(value, amount);
}

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

  // The real edges of the exemption, line by line: the exemption follows the goods, so
  // raw meat is exempt while the sausages beside it are not, and plain milk is exempt
  // while the yoghurt beside it is not.
  const TAX_BY_LINE: [string, "A" | "B"][] = [
    ["Beef rump steak", "A"],
    ["Beef mince", "A"],
    ["Tilapia, whole fresh", "A"],
    ["Eggs, tray of 30", "A"],
    ["Beef sausages", "B"],
    ["Smoked beef bacon", "B"],
    ["Inyange fresh milk", "A"],
    ["Inyange UHT milk", "A"],
    ["Inyange natural yoghurt", "B"],
    ["Gouda cheese", "B"],
    ["Butter, unsalted", "B"],
    ["Tomatoes", "A"],
    ["Minimex maize flour", "A"],
    ["Dry beans, red kidney", "A"],
    ["Kabuye white sugar", "B"],
    ["Table salt, iodised", "B"],
    ["Sunflower cooking oil", "B"],
    ["Sliced white bread", "B"],
    ["Always sanitary pads", "A"],
    ["Pampers baby diapers", "B"],
    ["Inyange still water", "B"],
    ["Primus", "B"],
  ];
  for (const [name, expected] of TAX_BY_LINE) {
    const matches = products.filter((p) => p.name === name);
    expect(matches.length, `no catalogue line named "${name}"`).toBeGreaterThanOrEqual(1);
    for (const p of matches) expect(p.taxTyCd, `${name} should be taxTyCd ${expected}`).toBe(expected);
  }
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

  // Discounts of both kinds are actually rung, at both line and basket level, so the
  // arithmetic checked below is reached rather than merely available.
  expect(sales.filter((s) => s.discountKind !== null).length).toBeGreaterThanOrEqual(1);
  expect(sales.some((s) => s.discountKind === "PERCENT")).toBe(true);
  expect(sales.some((s) => s.discountKind === "AMOUNT")).toBe(true);
  expect(lines.some((l) => l.discountKind === "PERCENT")).toBe(true);
  expect(lines.some((l) => l.discountKind === "AMOUNT")).toBe(true);
  expect(sales.every((s) => (s.discountKind === null) === (s.discountValue === null))).toBe(true);
  expect(lines.every((l) => (l.discountKind === null) === (l.discountValue === null))).toBe(true);

  for (const s of sales) {
    // Rebuild the whole receipt from its lines, including the basket discount's proration
    // between the exempt and standard-rated halves, and hold the seed to it to the cent.
    let exemptNet = new Prisma.Decimal(0);
    let standardNet = new Prisma.Decimal(0);
    let gross = new Prisma.Decimal(0);
    for (const l of s.lines) {
      const lineGross = dec(l.qty).times(l.prc).toDecimalPlaces(2);
      gross = gross.plus(lineGross);
      const net = lineGross.minus(discountOff(lineGross, l.discountKind, l.discountValue));
      if (l.taxTyCd === "A") exemptNet = exemptNet.plus(net);
      else standardNet = standardNet.plus(net);
    }
    const subtotal = exemptNet.plus(standardNet);
    const basketOff = discountOff(subtotal, s.discountKind, s.discountValue);
    const standardOff = subtotal.isZero()
      ? new Prisma.Decimal(0)
      : standardNet.div(subtotal).times(basketOff).toDecimalPlaces(2);

    expect(dec(s.totAmt).toString()).toBe(subtotal.minus(basketOff).toString());
    // VAT is carved out of the standard-rated lines only, net of their share of the
    // basket discount; exempt lines add no tax however the basket is discounted.
    expect(dec(s.totTaxAmt).toString()).toBe(vatOf(standardNet.minus(standardOff)).toString());
    // The EBM identity: the inclusive total is its base plus its tax, to the cent.
    expect(dec(s.totTaxblAmt).toString()).toBe(dec(s.totAmt).minus(s.totTaxAmt).toString());
    if (exemptNet.equals(subtotal)) expect(dec(s.totTaxAmt).toString()).toBe("0");

    // A discount only ever reduces the total, and never below nothing.
    if (basketOff.gt(0) || s.lines.some((l) => l.discountValue !== null)) {
      expect(dec(s.totAmt).lt(gross)).toBe(true);
    }
    expect(dec(s.totAmt).gt(0)).toBe(true);

    // Tenders settle the sale exactly.
    const tendered = s.tenders.reduce((t, x) => t.plus(x.amount), new Prisma.Decimal(0));
    expect(tendered.toString()).toBe(dec(s.totAmt).toString());

    // Only a weighed item can be sold in a fraction of a unit.
    for (const l of s.lines) {
      if (!dec(l.qty).isInteger()) expect(weighedIds.has(l.productId)).toBe(true);
      expect(dec(l.qty).gt(0)).toBe(true);
    }
  }

  // A running promotion is charged at the till. Lines sold inside an active window pay
  // the promotional price, not the shelf price.
  const promotions = await prisma.promotion.findMany({ where: { orgId } });
  expect(promotions.length).toBeGreaterThanOrEqual(1);
  let promotedLines = 0;
  for (const promo of promotions) {
    const shelf = await prisma.product.findUniqueOrThrow({ where: { id: promo.productId } });
    // If the promotion matched the shelf price, the assertion below would prove nothing.
    expect(dec(promo.price).equals(shelf.unitPrice)).toBe(false);
    for (const s of sales) {
      const inWindow = s.soldAt >= promo.startsAt && s.soldAt < promo.endsAt;
      for (const l of s.lines.filter((x) => x.productId === promo.productId)) {
        const expected = inWindow ? dec(promo.price) : dec(shelf.unitPrice);
        expect(dec(l.prc).toString()).toBe(expected.toString());
        if (inWindow) promotedLines++;
      }
    }
  }
  expect(promotedLines).toBeGreaterThanOrEqual(1);

  // One voided basket, so the lane screens have a void to render.
  expect(sales.filter((s) => s.voidedAt !== null).length).toBeGreaterThanOrEqual(1);
});

test("costPrice is derived from the ex-VAT price, not the shelf price", async () => {
  // User decision 2026-11-21: costPrice is ex-VAT. On band B the old inclusive
  // basis overstated the cost base by 18/118 = 15.25% of shelf.
  // Household carries a 0.25 margin; these three items are standard-rated.
  const { orgId } = await seedOrg();
  await seedGrocery(orgId, prisma);
  const cases = [
    { name: "Omo washing powder", pack: "1 kg", shelf: 3500, exVat: "2966.10", cost: "2224.58" },
    { name: "Sunlight laundry bar", pack: "800 g", shelf: 1500, exVat: "1271.19", cost: "953.39" },
    { name: "Dishwashing liquid", pack: "750 ml", shelf: 2200, exVat: "1864.41", cost: "1398.31" },
  ];
  for (const c of cases) {
    expect(exVatPrice(c.shelf, false).toFixed(2)).toBe(c.exVat);
    const p = await prisma.product.findFirstOrThrow({
      where: { orgId, name: c.name, packSize: c.pack },
    });
    expect(p.taxTyCd).toBe("B");
    expect(p.unitPrice.toFixed(2)).toBe(c.shelf.toFixed(2));
    expect(p.costPrice!.toFixed(2)).toBe(c.cost);
    // and the old basis is now provably not what we store
    expect(p.costPrice!.toFixed(2)).not.toBe((c.shelf * 0.75).toFixed(2));
  }
});

test("an exempt product's costPrice is unchanged by the ex-VAT rule", async () => {
  // Band A has no VAT to remove, so Produce's 0.30 margin applies to the shelf price.
  const { orgId } = await seedOrg();
  await seedGrocery(orgId, prisma);
  expect(exVatPrice(1200, true).toFixed(2)).toBe("1200.00");
  const tomatoes = await prisma.product.findFirstOrThrow({ where: { orgId, name: "Tomatoes" } });
  expect(tomatoes.taxTyCd).toBe("A");
  expect(tomatoes.costPrice!.toFixed(2)).toBe("840.00");
});

test("every standard-rated product costs less than its ex-VAT price", async () => {
  // the invariant the rule exists to protect: cost < ex-VAT < shelf, for all band B.
  // Expected ex-VAT is computed here with the test's own vatOf, not the seed's.
  const { orgId } = await seedOrg();
  await seedGrocery(orgId, prisma);
  const standard = await prisma.product.findMany({ where: { orgId, taxTyCd: "B" } });
  expect(standard.length).toBeGreaterThan(20);
  for (const p of standard) {
    const exVat = dec(p.unitPrice).minus(vatOf(dec(p.unitPrice)));
    expect(exVatPrice(Number(p.unitPrice), false).toString()).toBe(exVat.toString());
    expect(Number(p.costPrice)).toBeLessThan(Number(exVat));
    expect(Number(exVat)).toBeLessThan(Number(p.unitPrice));
  }
});
