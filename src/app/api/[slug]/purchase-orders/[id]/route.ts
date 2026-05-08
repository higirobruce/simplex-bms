import { prisma } from "@/lib/prisma";
import {
  getTenantContext,
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { PurchaseOrderPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(
  _req: Request,
  { params }: { params: { id: string } }
) {
  try {
    const { orgId } = await getTenantContext();
    const order = await prisma.purchaseOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: {
        vendor: true,
        lines: { include: { product: { select: { sku: true, name: true } } } },
        receipts: {
          include: {
            location: { select: { name: true } },
            lines: true,
          },
          orderBy: { createdAt: "desc" },
        },
      },
    });
    if (!order) return errorResponse("Purchase order not found", 404);
    return NextResponse.json(order);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(
  req: Request,
  { params }: { params: { id: string } }
) {
  try {
    const { orgId, userId } = await requirePermission("procurement:write");
    const body = await req.json();
    const parsed = PurchaseOrderPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.purchaseOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: { lines: true },
    });
    if (!existing) return errorResponse("Purchase order not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft orders can be edited", 422);
    }

    const { lines, ...rest } = parsed.data;
    const subtotal = lines
      ? lines.reduce((s, l) => s + l.qty * l.unitPrice, 0)
      : Number(existing.subtotal);

    const updated = await prisma.$transaction(async (tx) => {
      if (lines) {
        await tx.purchaseOrderLine.deleteMany({
          where: { orderId: params.id },
        });
        await tx.purchaseOrderLine.createMany({
          data: lines.map((l) => ({
            orderId: params.id,
            productId: l.productId,
            qty: l.qty,
            unitPrice: l.unitPrice,
            amount: l.qty * l.unitPrice,
          })),
        });
      }
      return tx.purchaseOrder.update({
        where: { id: params.id },
        data: {
          ...(rest.vendorId && { vendorId: rest.vendorId }),
          ...(rest.expectedDate !== undefined && {
            expectedDate: rest.expectedDate ? new Date(rest.expectedDate) : null,
          }),
          ...(rest.notes !== undefined && { notes: rest.notes || null }),
          subtotal,
          total: subtotal,
        },
        include: { lines: true, vendor: true },
      });
    });

    await logAudit({
      orgId,
      userId,
      action: "UPDATE",
      entityType: "PurchaseOrder",
      entityId: updated.id,
      before: existing,
      after: updated,
    });

    return NextResponse.json(updated);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function DELETE(
  _req: Request,
  { params }: { params: { id: string } }
) {
  try {
    const { orgId, userId } = await requirePermission("procurement:write");
    const existing = await prisma.purchaseOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
    });
    if (!existing) return errorResponse("Purchase order not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft orders can be deleted", 422);
    }
    await prisma.purchaseOrder.update({
      where: { id: params.id },
      data: { deletedAt: new Date() },
    });
    await logAudit({
      orgId,
      userId,
      action: "DELETE",
      entityType: "PurchaseOrder",
      entityId: params.id,
      before: existing,
    });
    return NextResponse.json({ ok: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
