import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

test("a batch carries a best-before date", async () => {
  const { orgId, productId } = await seedOrg();
  const b = await prisma.batch.create({
    data: { productId, lot: "L-2026-11", bestBefore: new Date("2026-12-24"), orgId },
  });
  expect(b.bestBefore?.toISOString()).toContain("2026-12-24");
});

test("a write-off records a fractional quantity and a reason", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  const w = await prisma.stockWriteOff.create({
    data: { productId, locationId, qty: "2.350", reason: "EXPIRY", orgId },
  });
  expect(w.qty.toString()).toBe("2.35");
  expect(w.reason).toBe("EXPIRY");
});

test("a stock movement starts unsynced to EBM", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  const m = await prisma.stockMovement.create({
    data: { productId, toLocationId: locationId, qty: "1", type: "RECEIPT", orgId },
  });
  expect(m.ebmSyncState).toBe("PENDING");
});

test("deleting a batch does not delete its movements", async () => {
  // batch is a label on stock, not its owner
  const { orgId, productId, locationId } = await seedOrg();
  const b = await prisma.batch.create({ data: { productId, lot: "L2", orgId } });
  const m = await prisma.stockMovement.create({
    data: { productId, toLocationId: locationId, qty: "1", type: "RECEIPT", orgId, batchId: b.id },
  });
  await prisma.batch.delete({ where: { id: b.id } });
  const still = await prisma.stockMovement.findUnique({ where: { id: m.id } });
  expect(still).not.toBeNull();
  expect(still!.batchId).toBeNull();
});

test("a write-off quantity must be positive", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  const make = (qty: string) =>
    prisma.stockWriteOff.create({ data: { productId, locationId, qty, reason: "DAMAGE", orgId } });
  // CHECK violations surface as PrismaClientUnknownRequestError with the Postgres text in .message
  await expect(make("0")).rejects.toThrow(/stockwriteoff_qty_positive/);
  await expect(make("-1.500")).rejects.toThrow(/stockwriteoff_qty_positive/);
  expect((await make("0.001")).qty.toString()).toBe("0.001");
});
