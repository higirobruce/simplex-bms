import { prisma } from "@/lib/prisma";
import {
  getTenantContext,
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { SalesOrderPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(
  _req: Request,
  { params }: { params: { id: string } }
) {
  try {
    const { orgId } = await getTenantContext();
    const order = await prisma.salesOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: {
        customer: true,
        lines: { include: { product: { select: { sku: true, name: true } } } },
        deliveryNotes: {
          include: {
            location: { select: { name: true } },
            lines: true,
            invoice: { select: { id: true, invoiceNo: true, status: true } },
          },
          orderBy: { createdAt: "desc" },
        },
      },
    });
    if (!order) return errorResponse("Sales order not found", 404);
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
    const { orgId, userId } = await requirePermission("sales:write");
    const body = await req.json();
    const parsed = SalesOrderPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.salesOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: { lines: true },
    });
    if (!existing) return errorResponse("Sales order not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft orders can be edited", 422);
    }

    const { lines, ...rest } = parsed.data;
    const subtotal = lines
      ? lines.reduce((s, l) => s + l.qty * l.unitPrice, 0)
      : Number(existing.subtotal);

    const updated = await prisma.$transaction(async (tx) => {
      if (lines) {
        await tx.salesOrderLine.deleteMany({ where: { orderId: params.id } });
        await tx.salesOrderLine.createMany({
          data: lines.map((l) => ({
            orderId: params.id,
            productId: l.productId,
            qty: l.qty,
            unitPrice: l.unitPrice,
            amount: l.qty * l.unitPrice,
          })),
        });
      }
      return tx.salesOrder.update({
        where: { id: params.id },
        data: {
          ...(rest.customerId && { customerId: rest.customerId }),
          ...(rest.expectedDate !== undefined && {
            expectedDate: rest.expectedDate ? new Date(rest.expectedDate) : null,
          }),
          ...(rest.notes !== undefined && { notes: rest.notes || null }),
          subtotal,
          total: subtotal,
        },
        include: { lines: true, customer: true },
      });
    });

    await logAudit({
      orgId,
      userId,
      action: "UPDATE",
      entityType: "SalesOrder",
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
    const { orgId, userId } = await requirePermission("sales:write");
    const existing = await prisma.salesOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
    });
    if (!existing) return errorResponse("Sales order not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft orders can be deleted", 422);
    }
    await prisma.salesOrder.update({
      where: { id: params.id },
      data: { deletedAt: new Date() },
    });
    await logAudit({
      orgId,
      userId,
      action: "DELETE",
      entityType: "SalesOrder",
      entityId: params.id,
      before: existing,
    });
    return NextResponse.json({ ok: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
