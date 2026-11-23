import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

test("two SALE movements for one product and one sale are refused", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  const make = () =>
    prisma.stockMovement.create({
      data: { productId, fromLocationId: locationId, qty: "1", type: "SALE", referenceId: "sale-1", orgId },
    });
  await make();
  await expect(make()).rejects.toThrow(/referenceId.*productId/);
});

test("two GRN movements for one product in different batches are still allowed", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  const b1 = await prisma.batch.create({ data: { productId, lot: "A", orgId } });
  const b2 = await prisma.batch.create({ data: { productId, lot: "B", orgId } });
  for (const batchId of [b1.id, b2.id]) {
    await prisma.stockMovement.create({
      data: { productId, toLocationId: locationId, qty: "5", type: "PURCHASE", referenceId: "grn-1", batchId, orgId },
    });
  }
  expect(await prisma.stockMovement.count({ where: { referenceId: "grn-1", productId } })).toBe(2);
});

test("two PURCHASE movements for one product without batch are allowed", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  for (let i = 0; i < 2; i++) {
    await prisma.stockMovement.create({
      data: { productId, toLocationId: locationId, qty: "5", type: "PURCHASE", referenceId: "grn-1", orgId },
    });
  }
  expect(await prisma.stockMovement.count({ where: { referenceId: "grn-1", productId } })).toBe(2);
});

test("two SALE movements with same referenceId but different productId are allowed", async () => {
  const { orgId, locationId } = await seedOrg();
  const p1 = await prisma.product.create({ data: { sku: "P1-SKU", name: "P1", unitPrice: "100", orgId } });
  const p2 = await prisma.product.create({ data: { sku: "P2-SKU", name: "P2", unitPrice: "100", orgId } });
  for (const pid of [p1.id, p2.id]) {
    await prisma.stockMovement.create({
      data: { productId: pid, fromLocationId: locationId, qty: "1", type: "SALE", referenceId: "sale-1", orgId },
    });
  }
  expect(await prisma.stockMovement.count({ where: { referenceId: "sale-1" } })).toBe(2);
});

test("two SALE movements with same productId but different referenceId are allowed", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  for (let i = 0; i < 2; i++) {
    await prisma.stockMovement.create({
      data: { productId, fromLocationId: locationId, qty: "1", type: "SALE", referenceId: `sale-${i}`, orgId },
    });
  }
  expect(await prisma.stockMovement.count({ where: { productId, type: "SALE" } })).toBe(2);
});

test("a SALE movement and a PURCHASE movement with same referenceId and productId are allowed", async () => {
  const { orgId, productId, locationId } = await seedOrg();
  await prisma.stockMovement.create({
    data: { productId, fromLocationId: locationId, qty: "1", type: "SALE", referenceId: "ref-1", orgId },
  });
  await prisma.stockMovement.create({
    data: { productId, toLocationId: locationId, qty: "1", type: "PURCHASE", referenceId: "ref-1", orgId },
  });
  expect(await prisma.stockMovement.count({ where: { referenceId: "ref-1", productId } })).toBe(2);
});

test("delivery note with one SALE movement per product is allowed", async () => {
  const { orgId, locationId } = await seedOrg();
  const p1 = await prisma.product.create({ data: { sku: "P1-SKU", name: "P1", unitPrice: "100", orgId } });
  const p2 = await prisma.product.create({ data: { sku: "P2-SKU", name: "P2", unitPrice: "100", orgId } });
  const noteId = "dn-123";
  for (const pid of [p1.id, p2.id]) {
    await prisma.stockMovement.create({
      data: { productId: pid, fromLocationId: locationId, qty: "1", type: "SALE", referenceId: noteId, orgId },
    });
  }
  expect(await prisma.stockMovement.count({ where: { referenceId: noteId, type: "SALE" } })).toBe(2);
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
