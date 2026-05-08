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
    const existing = await prisma.goodsReceiptNote.findFirst({
      where: { id: params.id, orgId },
    });
    if (!existing) return errorResponse("Goods receipt not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft goods receipts can be cancelled", 422);
    }
    const updated = await prisma.goodsReceiptNote.update({
      where: { id: params.id },
      data: { status: "CANCELLED" },
    });
    await logAudit({
      orgId,
      userId,
      action: "CANCEL",
      entityType: "GoodsReceiptNote",
      entityId: updated.id,
      before: existing,
      after: updated,
    });
    return NextResponse.json(updated);
  } catch (error) {
    return handleRouteError(error);
  }
}
