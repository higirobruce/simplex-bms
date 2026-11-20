import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";
import { Prisma } from "@prisma/client";
import { formatQty, toQty } from "@/lib/qty";

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

test("a fractional sales order line can be partly delivered", async () => {
  const { productId, orgId, userId } = await seedOrg();
  const customer = await prisma.customer.create({ data: { name: "C", orgId } });
  const order = await prisma.salesOrder.create({
    data: { orderNo: "SO-1", customerId: customer.id, orgId, createdById: userId },
  });
  const line = await prisma.salesOrderLine.create({
    data: { orderId: order.id, productId, qty: "2.5", unitPrice: "10.00", amount: "25.00" },
  });
  // Same write the delivery-note approval performs.
  await prisma.salesOrderLine.update({
    where: { id: line.id },
    data: { qtyDelivered: { increment: new Prisma.Decimal("1.25") } },
  });
  const after = await prisma.salesOrderLine.findUniqueOrThrow({ where: { id: line.id } });
  expect(after.qtyDelivered.toString()).toBe("1.25");
  expect(toQty(after.qtyDelivered).gte(after.qty)).toBe(false);
});
