import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { OrgPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET() {
  try {
    const { orgId } = await getTenantContext();
    const org = await prisma.org.findUnique({
      where: { id: orgId },
      select: { id: true, slug: true, name: true, logo: true, currency: true, timezone: true, createdAt: true },
    });
    if (!org) return errorResponse("Organisation not found", 404);
    return NextResponse.json(org);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("org:write");
    const body = await req.json();
    const parsed = OrgPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const before = await prisma.org.findUnique({
      where: { id: orgId },
      select: { id: true, slug: true, name: true, logo: true, currency: true, timezone: true },
    });
    if (!before) return errorResponse("Organisation not found", 404);

    const after = await prisma.org.update({
      where: { id: orgId },
      data: parsed.data,
      select: { id: true, slug: true, name: true, logo: true, currency: true, timezone: true, createdAt: true },
    });

    await logAudit({ orgId, userId, action: "UPDATE", entityType: "Org", entityId: orgId, before, after });
    return NextResponse.json(after);
  } catch (error) {
    return handleRouteError(error);
  }
}
