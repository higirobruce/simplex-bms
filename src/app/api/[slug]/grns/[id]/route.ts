import { prisma } from "@/lib/prisma";
import {
  getTenantContext,
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(
  _req: Request,
  { params }: { params: { id: string } }
) {
  try {
    const { orgId } = await getTenantContext();
    const note = await prisma.goodsReceiptNote.findFirst({
      where: { id: params.id, orgId },
      include: {
        order: {
          include: {
            vendor: true,
            lines: { include: { product: true } },
          },
        },
        location: true,
        lines: { include: { product: true } },
      },
    });
    if (!note) return errorResponse("Goods receipt not found", 404);
    return NextResponse.json(note);
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
    const existing = await prisma.goodsReceiptNote.findFirst({
      where: { id: params.id, orgId },
    });
    if (!existing) return errorResponse("Goods receipt not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft goods receipts can be deleted", 422);
    }
    await prisma.goodsReceiptNote.delete({ where: { id: params.id } });
    await logAudit({
      orgId,
      userId,
      action: "DELETE",
      entityType: "GoodsReceiptNote",
      entityId: params.id,
      before: existing,
    });
    return NextResponse.json({ ok: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
