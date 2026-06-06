import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError, errorResponse } from "@/lib/api";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

// Restore a soft-deleted shop (clear deletedAt). Its users regain access.
export async function POST(_req: Request, { params }: { params: { id: string } }) {
  try {
    const { userId } = await requireSuperAdmin();
    const org = await prisma.org.findUnique({
      where: { id: params.id },
      select: { id: true, slug: true, name: true, deletedAt: true },
    });
    if (!org) return errorResponse("Shop not found", 404);
    if (!org.deletedAt) return errorResponse("Shop is not removed", 409);

    const after = await prisma.org.update({
      where: { id: params.id },
      data: { deletedAt: null },
      select: { id: true, slug: true, name: true, status: true, createdAt: true },
    });

    await logAudit({
      orgId: params.id,
      userId,
      action: "UPDATE",
      entityType: "Org",
      entityId: params.id,
      before: { deletedAt: org.deletedAt },
      after: { ...after, restored: true },
    });
    return NextResponse.json(after);
  } catch (error) {
    return handleRouteError(error);
  }
}
