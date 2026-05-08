import { prisma } from "@/lib/prisma";
import {
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function POST(
  _req: Request,
  { params }: { params: { id: string } }
) {
  try {
    const { orgId, userId } = await requirePermission("sales:approve");

    const note = await prisma.deliveryNote.findFirst({
      where: { id: params.id, orgId },
      include: {
        lines: true,
        order: {
          include: {
            lines: { include: { product: true } },
            customer: true,
          },
        },
      },
    });
    if (!note) return errorResponse("Delivery note not found", 404);
    if (note.status !== "DRAFT") {
      return errorResponse(
        `Delivery note is already ${note.status.toLowerCase()}`,
        422
      );
    }

    // Aggregate qty per product for stock check.
    const productTotals = new Map<string, number>();
    for (const l of note.lines) {
      productTotals.set(
        l.productId,
        (productTotals.get(l.productId) || 0) + l.qty
      );
    }

    // Hard-block validation: every product must have enough stock at the source warehouse.
    const stockLevels = await prisma.stockLevel.findMany({
      where: {
        locationId: note.locationId,
        productId: { in: Array.from(productTotals.keys()) },
      },
    });
    const stockMap = new Map(stockLevels.map((s) => [s.productId, s.qty]));

    const insufficient: string[] = [];
    for (const [productId, qtyNeeded] of Array.from(productTotals.entries())) {
      const onHand = stockMap.get(productId) ?? 0;
      if (onHand < qtyNeeded) {
        const product = note.order.lines.find(
          (ol) => ol.productId === productId
        )?.product;
        insufficient.push(
          `${product?.name ?? productId}: need ${qtyNeeded}, on hand ${onHand}`
        );
      }
    }
    if (insufficient.length > 0) {
      return errorResponse(
        `Insufficient stock at source warehouse — ${insufficient.join("; ")}`,
        422
      );
    }

    // Atomic apply: decrement stock, write movements, update SO line qtyDelivered,
    // possibly mark SO as FULFILLED, auto-create draft invoice.
    const result = await prisma.$transaction(async (tx) => {
      // 1. Decrement stock + write movements
      for (const [productId, qty] of Array.from(productTotals.entries())) {
        await tx.stockLevel.update({
          where: {
            productId_locationId: { productId, locationId: note.locationId },
          },
          data: { qty: { decrement: qty } },
        });
        await tx.stockMovement.create({
          data: {
            orgId,
            productId,
            fromLocationId: note.locationId,
            qty,
            type: "SALE",
            reason: `Delivery ${note.noteNo}`,
            referenceId: note.id,
          },
        });
      }

      // 2. Update SO line qtyDelivered for each DN line linked to an order line
      for (const dnLine of note.lines) {
        if (dnLine.orderLineId) {
          await tx.salesOrderLine.update({
            where: { id: dnLine.orderLineId },
            data: { qtyDelivered: { increment: dnLine.qty } },
          });
        }
      }

      // 3. Reload SO lines to check if fully delivered
      const refreshed = await tx.salesOrder.findUnique({
        where: { id: note.orderId },
        include: { lines: true },
      });
      const fully =
        refreshed?.lines.every((l) => l.qtyDelivered >= l.qty) ?? false;
      if (fully) {
        await tx.salesOrder.update({
          where: { id: note.orderId },
          data: { status: "FULFILLED" },
        });
      }

      // 4. Auto-create draft invoice using order line prices
      const invoiceNoRows = await tx.$queryRaw<[{ lastSeq: number }]>`
        WITH max_existing AS (
          SELECT COALESCE(MAX(CAST(SUBSTRING("invoiceNo" FROM 5) AS INTEGER)), 0) AS n
          FROM "Invoice"
          WHERE "orgId" = ${orgId} AND "invoiceNo" ~ '^INV-[0-9]+$'
        )
        INSERT INTO "InvoiceSequence" ("orgId", "lastSeq")
        SELECT ${orgId}, n + 1 FROM max_existing
        ON CONFLICT ("orgId") DO UPDATE
          SET "lastSeq" = GREATEST(
            "InvoiceSequence"."lastSeq" + 1,
            (SELECT n + 1 FROM max_existing)
          )
        RETURNING "lastSeq"
      `;
      const invoiceNo = `INV-${String(invoiceNoRows[0].lastSeq).padStart(4, "0")}`;

      // Look up products for fallback pricing when DN line is unlinked
      const productLookup = await tx.product.findMany({
        where: {
          id: { in: Array.from(productTotals.keys()) },
          orgId,
        },
        select: { id: true, name: true, unitPrice: true },
      });
      const productById = new Map(productLookup.map((p) => [p.id, p]));

      // Build invoice line items from DN lines using SO line unit prices
      const invoiceLines = note.lines.map((dnLine) => {
        const orderLine = note.order.lines.find(
          (ol) => ol.id === dnLine.orderLineId
        );
        const fallbackProduct = productById.get(dnLine.productId);
        const unitPrice = orderLine
          ? Number(orderLine.unitPrice)
          : Number(fallbackProduct?.unitPrice ?? 0);
        const productName =
          orderLine?.product.name ?? fallbackProduct?.name ?? null;
        return {
          productId: dnLine.productId,
          description: productName,
          qty: dnLine.qty,
          unitPrice,
          amount: dnLine.qty * unitPrice,
        };
      });

      const subtotal = invoiceLines.reduce((s, l) => s + l.amount, 0);

      const dueDate = new Date();
      dueDate.setDate(dueDate.getDate() + 30);

      const invoice = await tx.invoice.create({
        data: {
          orgId,
          invoiceNo,
          customerId: note.order.customerId,
          status: "DRAFT",
          subtotal,
          total: subtotal,
          dueDate,
          notes: `Auto-generated from delivery ${note.noteNo}`,
          lineItems: { create: invoiceLines },
        },
      });

      // 5. Mark DN approved + link to invoice
      const updated = await tx.deliveryNote.update({
        where: { id: note.id },
        data: {
          status: "APPROVED",
          approvedAt: new Date(),
          approvedById: userId,
          invoiceId: invoice.id,
        },
        include: { invoice: true, lines: true, location: true },
      });

      return { note: updated, invoice };
    });

    await logAudit({
      orgId,
      userId,
      action: "APPROVE",
      entityType: "DeliveryNote",
      entityId: result.note.id,
      before: note,
      after: result.note,
    });

    return NextResponse.json(result.note);
  } catch (error) {
    return handleRouteError(error);
  }
}
