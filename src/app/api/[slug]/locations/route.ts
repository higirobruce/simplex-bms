import { prisma } from "@/lib/prisma";
import {
  getTenantContext,
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { LocationCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET() {
  try {
    const { orgId } = await getTenantContext();
    const locations = await prisma.location.findMany({
      where: { orgId },
      orderBy: { createdAt: "asc" },
      include: {
        stockLevels: { select: { qty: true } },
      },
    });

    // Compute total qty per location and shape into a tree.
    const enriched = locations.map((l) => ({
      id: l.id,
      name: l.name,
      code: l.code,
      address: l.address,
      parentId: l.parentId,
      orgId: l.orgId,
      createdAt: l.createdAt,
      directQty: l.stockLevels.reduce((sum, s) => sum + s.qty, 0),
    }));

    return NextResponse.json(enriched);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("locations:write");
    const body = await req.json();
    const parsed = LocationCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const { name, code, address, parentId } = parsed.data;

    if (parentId) {
      const parent = await prisma.location.findFirst({
        where: { id: parentId, orgId },
      });
      if (!parent) return errorResponse("Parent location not found", 404);
    }

    const location = await prisma.location.create({
      data: {
        orgId,
        name,
        code: code || null,
        address: address || null,
        parentId: parentId || null,
      },
    });

    await logAudit({
      orgId,
      userId,
      action: "CREATE",
      entityType: "Location",
      entityId: location.id,
      after: location,
    });

    return NextResponse.json(location);
  } catch (error) {
    return handleRouteError(error);
  }
}
