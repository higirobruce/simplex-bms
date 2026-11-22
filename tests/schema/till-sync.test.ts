import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

test("two SALE movements for one product and one sale are refused", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  const make = () =>
    prisma.stockMovement.create({
      data: { productId, fromLocationId: locationId, qty: "1", type: "SALE", referenceId: "sale-1", orgId },
    });
  await make();
  await expect(make()).rejects.toThrow(/StockMovement_sale_ref_product_key|Unique constraint/);
});

test("two GRN movements for one product in different batches are still allowed", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  const b1 = await prisma.batch.create({ data: { productId, lot: "A", orgId } });
  const b2 = await prisma.batch.create({ data: { productId, lot: "B", orgId } });
  for (const batchId of [b1.id, b2.id]) {
    await prisma.stockMovement.create({
      data: { productId, toLocationId: locationId, qty: "5", type: "RECEIPT", referenceId: "grn-1", batchId, orgId },
    });
  }
  expect(await prisma.stockMovement.count({ where: { referenceId: "grn-1", productId } })).toBe(2);
});

test("a lane token hash is unique", async () => {
  const { orgId, locationId } = await seedOrg();
  const lane = await prisma.lane.create({ data: { code: "L1", name: "Lane 1", locationId, orgId } });
  const make = () => prisma.laneToken.create({ data: { laneId: lane.id, tokenHash: "ab".repeat(32), orgId } });
  await make();
  await expect(make()).rejects.toThrow(/Unique constraint/);
});

test("a cashier can hold a pin hash", async () => {
  const { orgId } = await seedOrg();
  const u = await prisma.user.create({
    data: { email: "c@test.local", password: "x", role: "CASHIER", pinHash: "$argon2id$x", orgId },
  });
  expect(u.role).toBe("CASHIER");
  expect(u.pinHash).toBe("$argon2id$x");
});
