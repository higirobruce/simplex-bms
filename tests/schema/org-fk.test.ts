import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

// Every tenant-scoped row must point at a real Org: a sale filed under a non-existent (or
// mistyped) org is revenue under the wrong taxpayer, and Phase 2 writes sales from outside
// the request context that normally supplies orgId.
const BOGUS = "no-such-org";

async function seed() {
  const s = await seedOrg();
  const lane = await prisma.lane.create({
    data: { code: "L1", name: "Lane 1", locationId: s.locationId, orgId: s.orgId },
  });
  const shift = await prisma.shift.create({
    data: { laneId: lane.id, openedById: s.userId, openingFloat: "0", orgId: s.orgId },
  });
  const sale = await prisma.sale.create({
    data: {
      shiftId: shift.id, laneId: lane.id, invcNo: 1,
      totAmt: "100", totTaxblAmt: "84.75", totTaxAmt: "15.25", orgId: s.orgId,
    },
  });
  return { ...s, lane, shift, sale };
}

const FK = /Foreign key constraint/;

test("each new model rejects an orgId that is not an Org", async () => {
  const { productId, locationId, userId, lane, shift, sale } = await seed();
  const now = new Date();
  const attempts: Record<string, () => Promise<unknown>> = {
    Batch: () => prisma.batch.create({ data: { productId, lot: "X", orgId: BOGUS } }),
    StockWriteOff: () =>
      prisma.stockWriteOff.create({ data: { productId, locationId, qty: "1", reason: "DAMAGE", orgId: BOGUS } }),
    Promotion: () =>
      prisma.promotion.create({
        data: { productId, price: "1", startsAt: now, endsAt: new Date(now.getTime() + 1000), orgId: BOGUS },
      }),
    PriceChange: () =>
      prisma.priceChange.create({ data: { productId, oldPrice: "1", newPrice: "2", changedById: userId, orgId: BOGUS } }),
    Lane: () => prisma.lane.create({ data: { code: "L9", name: "x", locationId, orgId: BOGUS } }),
    Shift: () =>
      prisma.shift.create({ data: { laneId: lane.id, openedById: userId, openingFloat: "0", orgId: BOGUS } }),
    Sale: () =>
      prisma.sale.create({
        data: {
          shiftId: shift.id, laneId: lane.id, invcNo: 2,
          totAmt: "1", totTaxblAmt: "1", totTaxAmt: "0", orgId: BOGUS,
        },
      }),
    SaleLine: () =>
      prisma.saleLine.create({
        data: { saleId: sale.id, productId, itemSeq: 1, qty: "1", prc: "1", taxTyCd: "B", orgId: BOGUS },
      }),
    Tender: () => prisma.tender.create({ data: { saleId: sale.id, seq: 0, pmtTyCd: "CASH", amount: "1", orgId: BOGUS } }),
  };
  for (const [name, attempt] of Object.entries(attempts)) {
    await expect(attempt(), name).rejects.toThrow(FK);
  }
});

test("sales are indexed by shift", async () => {
  const rows = await prisma.$queryRaw<{ indexdef: string }[]>`
    select indexdef from pg_indexes where tablename = 'Sale' and indexname = 'Sale_shiftId_idx'`;
  expect(rows).toHaveLength(1);
});
