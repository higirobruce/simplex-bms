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
    });
    if (!existing) return errorResponse("Sales order not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft orders can be confirmed", 422);
    }

    const updated = await prisma.salesOrder.update({
      where: { id: params.id },
      data: { status: "CONFIRMED" },
    });

    await logAudit({
      orgId,
      userId,
      action: "CONFIRM",
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
