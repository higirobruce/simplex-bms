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
    const { orgId, userId } = await requirePermission("procurement:approve");

    const note = await prisma.goodsReceiptNote.findFirst({
      where: { id: params.id, orgId },
      include: { lines: true, order: { include: { lines: true } } },
    });
    if (!note) return errorResponse("Goods receipt not found", 404);
    if (note.status !== "DRAFT") {
      return errorResponse(
        `Goods receipt is already ${note.status.toLowerCase()}`,
        422
      );
    }

    // Aggregate qty per product for stock increment.
    const productTotals = new Map<string, number>();
    for (const l of note.lines) {
      productTotals.set(
        l.productId,
        (productTotals.get(l.productId) || 0) + l.qty
      );
    }

    const result = await prisma.$transaction(async (tx) => {
      // 1. Increment stock + write movements
      for (const [productId, qty] of Array.from(productTotals.entries())) {
        await tx.stockLevel.upsert({
          where: {
            productId_locationId: { productId, locationId: note.locationId },
          },
          update: { qty: { increment: qty } },
          create: { productId, locationId: note.locationId, qty },
        });
        await tx.stockMovement.create({
          data: {
            orgId,
            productId,
            toLocationId: note.locationId,
            qty,
            type: "PURCHASE",
            reason: `Goods receipt ${note.noteNo}`,
            referenceId: note.id,
          },
        });
      }

      // 2. Update PO line qtyReceived
      for (const grnLine of note.lines) {
        if (grnLine.orderLineId) {
          await tx.purchaseOrderLine.update({
            where: { id: grnLine.orderLineId },
            data: { qtyReceived: { increment: grnLine.qty } },
          });
        }
      }

      // 3. If all PO lines fully received, mark PO as RECEIVED
      const refreshed = await tx.purchaseOrder.findUnique({
        where: { id: note.orderId },
        include: { lines: true },
      });
      const fully =
        refreshed?.lines.every((l) => l.qtyReceived >= l.qty) ?? false;
      if (fully) {
        await tx.purchaseOrder.update({
          where: { id: note.orderId },
          data: { status: "RECEIVED" },
        });
      }

      // 4. Mark GRN approved
      const updated = await tx.goodsReceiptNote.update({
        where: { id: note.id },
        data: {
          status: "APPROVED",
          approvedAt: new Date(),
          approvedById: userId,
        },
        include: { lines: true, location: true, order: true },
      });

      return updated;
    });

    await logAudit({
      orgId,
      userId,
      action: "APPROVE",
      entityType: "GoodsReceiptNote",
      entityId: result.id,
      before: note,
      after: result,
    });

    return NextResponse.json(result);
  } catch (error) {
    return handleRouteError(error);
  }
}
