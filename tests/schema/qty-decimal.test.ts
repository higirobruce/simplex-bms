import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";
import { formatQty } from "@/lib/qty";

test("a weighed quantity survives a round trip", async () => {
  const { productId, locationId } = await seedOrg();
  const level = await prisma.stockLevel.create({
    data: { productId, locationId, qty: "0.734" },
  });
  expect(level.qty.toString()).toBe("0.734");
});

test("the column scale is 3, so a fourth decimal does not survive", async () => {
  const { productId, locationId, orgId } = await seedOrg();
  const m = await prisma.stockMovement.create({
    data: { productId, toLocationId: locationId, qty: "1.2345", type: "RECEIPT", orgId },
  });
  expect(m.qty.toString()).not.toBe("1.2345");
  expect(m.qty.decimalPlaces()).toBeLessThanOrEqual(3);
});

test("integer quantities still read as whole numbers", async () => {
  const { productId, orgId } = await seedOrg();
  const customer = await prisma.customer.create({ data: { name: "C", orgId } });
  const invoice = await prisma.invoice.create({
    data: { invoiceNo: "INV-1", customerId: customer.id, dueDate: new Date(), orgId },
  });
  const line = await prisma.invoiceLineItem.create({
    data: { invoiceId: invoice.id, productId, qty: 3, unitPrice: "10.00", amount: "30.00" },
  });
  expect(formatQty(line.qty, false)).toBe("3");
});
