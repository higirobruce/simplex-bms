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
    const { orgId, userId } = await requirePermission("procurement:write");
    const existing = await prisma.purchaseOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: {
        receipts: { where: { status: "APPROVED" }, select: { id: true } },
      },
    });
    if (!existing) return errorResponse("Purchase order not found", 404);
    if (existing.status === "RECEIVED" || existing.status === "CANCELLED") {
      return errorResponse(
        `Order is already ${existing.status.toLowerCase()}`,
        422
      );
    }
    if (existing.receipts.length > 0) {
      return errorResponse(
        "Cannot cancel an order with approved goods receipts",
        422
      );
    }
    const updated = await prisma.purchaseOrder.update({
      where: { id: params.id },
      data: { status: "CANCELLED" },
    });
    await logAudit({
      orgId,
      userId,
      action: "CANCEL",
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
