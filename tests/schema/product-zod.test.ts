import { expect, test } from "vitest";
import { ProductCreateSchema, ProductPatchSchema, ProductImportRowSchema } from "@/lib/validations";
import { prisma, seedOrg } from "../helpers/db";

const base = { sku: "S1", name: "N", unitPrice: 100 };

test("taxTyCd E is rejected", () => {
  expect(ProductCreateSchema.safeParse({ ...base, taxTyCd: "E" }).success).toBe(false);
});

test("blank and whitespace barcodes become null", () => {
  for (const barcode of ["", "  "]) {
    const r = ProductCreateSchema.parse({ ...base, barcode });
    expect(r.barcode).toBeNull();
  }
  expect(ProductCreateSchema.parse({ ...base, barcode: " 123 " }).barcode).toBe("123");
});

test("shelfLifeDays 0, -1 and over the bound are rejected", () => {
  for (const shelfLifeDays of [0, -1, 3651, 2 ** 31]) {
    expect(ProductCreateSchema.safeParse({ ...base, shelfLifeDays }).success).toBe(false);
    expect(ProductPatchSchema.safeParse({ shelfLifeDays }).success).toBe(false);
  }
});

test("defaults apply when grocery fields are omitted", () => {
  const r = ProductCreateSchema.parse(base);
  expect(r).toMatchObject({ taxTyCd: "B", isWeighed: false, department: "General" });
});

test("a full valid payload round-trips", () => {
  const full = {
    ...base, barcode: "5901234123457", plu: "4664", isWeighed: true, taxTyCd: "A" as const,
    department: "Produce", packSize: "6x500ml", shelfLifeDays: 14,
  };
  expect(ProductCreateSchema.parse(full)).toMatchObject(full);
});

test("importer reads grocery columns and defaults blanks", () => {
  const r = ProductImportRowSchema.parse({
    sku: "S", name: "N", unitPrice: "100", barcode: "  ", plu: "", isWeighed: "yes",
    taxTyCd: "a", department: "", packSize: "", shelfLifeDays: "7",
  });
  expect(r).toMatchObject({ barcode: null, plu: null, isWeighed: true, taxTyCd: "A", department: "General", packSize: null, shelfLifeDays: 7 });
  const d = ProductImportRowSchema.parse({ sku: "S", name: "N", unitPrice: "100", taxTyCd: "", isWeighed: "", shelfLifeDays: "" });
  expect(d).toMatchObject({ taxTyCd: "B", isWeighed: false });
  expect(d.shelfLifeDays).toBeUndefined();
  expect(ProductImportRowSchema.safeParse({ sku: "S", name: "N", unitPrice: "1", taxTyCd: "E" }).success).toBe(false);
});

test("normalised blank barcodes do not collide in the unique index", async () => {
  const { orgId } = await seedOrg();
  for (const sku of ["B1", "B2"]) {
    const data = ProductCreateSchema.parse({ ...base, sku, barcode: "" });
    await prisma.product.create({ data: { orgId, ...data, unitPrice: "1.00", reorderLevel: "10" } });
  }
  expect(await prisma.product.count({ where: { orgId, barcode: null } })).toBeGreaterThanOrEqual(2);
});
