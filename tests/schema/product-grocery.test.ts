import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

test("tax type defaults to B rather than exempt", async () => {
  const { orgId } = await seedOrg();
  const p = await prisma.product.create({ data: { sku: "X1", name: "X", unitPrice: "1.00", orgId } });
  expect(p.taxTyCd).toBe("B"); // Review Focus #4
});

test("the four RRA tax types are all accepted", async () => {
  const { orgId } = await seedOrg();
  for (const t of ["A", "B", "C", "D"] as const) {
    const p = await prisma.product.create({
      data: { sku: `T${t}`, name: t, unitPrice: "1.00", orgId, taxTyCd: t },
    });
    expect(p.taxTyCd).toBe(t);
  }
});

test("barcode is unique per org but not across orgs", async () => {
  const { orgId, otherOrgId } = await seedOrg();
  await prisma.product.create({ data: { sku: "A", name: "A", unitPrice: "1.00", orgId, barcode: "5901234123457" } });
  await expect(
    prisma.product.create({ data: { sku: "B", name: "B", unitPrice: "1.00", orgId, barcode: "5901234123457" } })
  ).rejects.toThrow();
  const other = await prisma.product.create({
    data: { sku: "C", name: "C", unitPrice: "1.00", orgId: otherOrgId, barcode: "5901234123457" },
  });
  expect(other.barcode).toBe("5901234123457");
});

test("two weighed products may share a PLU prefix", async () => {
  // Review Focus #5: PLU is indexed, not unique; weight-embedded barcodes
  // are decoded to a PLU and must not be stored as product barcodes.
  const { orgId } = await seedOrg();
  await prisma.product.create({ data: { sku: "TOM", name: "Tomatoes", unitPrice: "1.00", orgId, plu: "4664", isWeighed: true } });
  const second = await prisma.product.create({
    data: { sku: "TOMV", name: "Tomatoes Vine", unitPrice: "1.00", orgId, plu: "46641", isWeighed: true },
  });
  expect(second.plu).toBe("46641");
});

test("barcode may be null for many products at once", async () => {
  const { orgId } = await seedOrg();
  await prisma.product.create({ data: { sku: "N1", name: "N1", unitPrice: "1.00", orgId } });
  const n2 = await prisma.product.create({ data: { sku: "N2", name: "N2", unitPrice: "1.00", orgId } });
  expect(n2.barcode).toBeNull();
});
