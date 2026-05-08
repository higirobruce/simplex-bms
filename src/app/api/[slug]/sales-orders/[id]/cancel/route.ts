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
    const { orgId, userId } = await requirePermission("sales:write");

    const existing = await prisma.salesOrder.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: {
        deliveryNotes: { where: { status: "APPROVED" }, select: { id: true } },
      },
    });
    if (!existing) return errorResponse("Sales order not found", 404);
    if (existing.status === "FULFILLED" || existing.status === "CANCELLED") {
      return errorResponse(`Order is already ${existing.status.toLowerCase()}`, 422);
    }
    if (existing.deliveryNotes.length > 0) {
      return errorResponse(
        "Cannot cancel an order with approved delivery notes",
        422
      );
    }

    const updated = await prisma.salesOrder.update({
      where: { id: params.id },
      data: { status: "CANCELLED" },
    });

    await logAudit({
      orgId,
      userId,
      action: "CANCEL",
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
