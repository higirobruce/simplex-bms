import { prisma } from "@/lib/prisma";
import {
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { LocationPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function PATCH(
  req: Request,
  { params }: { params: { id: string } }
) {
  try {
    const { orgId, userId } = await requirePermission("locations:write");
    const body = await req.json();
    const parsed = LocationPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.location.findFirst({
      where: { id: params.id, orgId },
    });
    if (!existing) return errorResponse("Location not found", 404);

    const { parentId } = parsed.data;
    if (parentId) {
      if (parentId === params.id) {
        return errorResponse("A location cannot be its own parent", 422);
      }
      // Walk up the proposed parent chain to detect cycles.
      let cursor: string | null = parentId;
      const seen = new Set<string>();
      while (cursor) {
        if (cursor === params.id) {
          return errorResponse(
            "Cannot move a location under one of its own descendants",
            422
          );
        }
        if (seen.has(cursor)) break;
        seen.add(cursor);
        const next: { parentId: string | null } | null =
          await prisma.location.findFirst({
            where: { id: cursor, orgId },
            select: { parentId: true },
          });
        cursor = next?.parentId ?? null;
      }
    }

    const updated = await prisma.location.update({
      where: { id: params.id },
      data: {
        ...parsed.data,
        parentId: parentId === undefined ? undefined : parentId || null,
      },
    });

    await logAudit({
      orgId,
      userId,
      action: "UPDATE",
      entityType: "Location",
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
    const { orgId, userId } = await requirePermission("locations:delete");

    const existing = await prisma.location.findFirst({
      where: { id: params.id, orgId },
      include: {
        children: { select: { id: true } },
        stockLevels: { select: { qty: true } },
      },
    });
    if (!existing) return errorResponse("Location not found", 404);

    if (existing.children.length > 0) {
      return errorResponse(
        "Cannot delete a location with sub-locations. Remove or reparent them first.",
        422
      );
    }

    const totalQty = existing.stockLevels.reduce((sum, s) => sum + s.qty, 0);
    if (totalQty > 0) {
      return errorResponse(
        "Cannot delete a location that still holds stock.",
        422
      );
    }

    await prisma.location.delete({ where: { id: params.id } });

    await logAudit({
      orgId,
      userId,
      action: "DELETE",
      entityType: "Location",
      entityId: params.id,
      before: existing,
    });

    return NextResponse.json({ ok: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
