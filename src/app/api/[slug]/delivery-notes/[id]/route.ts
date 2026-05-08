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
    const note = await prisma.deliveryNote.findFirst({
      where: { id: params.id, orgId },
      include: {
        order: {
          include: {
            customer: true,
            lines: { include: { product: true } },
          },
        },
        location: true,
        lines: { include: { product: true } },
        invoice: true,
      },
    });
    if (!note) return errorResponse("Delivery note not found", 404);
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
    const { orgId, userId } = await requirePermission("sales:write");
    const existing = await prisma.deliveryNote.findFirst({
      where: { id: params.id, orgId },
    });
    if (!existing) return errorResponse("Delivery note not found", 404);
    if (existing.status !== "DRAFT") {
      return errorResponse("Only draft delivery notes can be deleted", 422);
    }
    await prisma.deliveryNote.delete({ where: { id: params.id } });
    await logAudit({
      orgId,
      userId,
      action: "DELETE",
      entityType: "DeliveryNote",
      entityId: params.id,
      before: existing,
    });
    return NextResponse.json({ ok: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
