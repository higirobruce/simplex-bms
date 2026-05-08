import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { VendorPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId } = await getTenantContext();
    const vendor = await prisma.vendor.findFirst({ where: { id: params.id, orgId, deletedAt: null } });
    if (!vendor) return errorResponse("Vendor not found", 404);
    return NextResponse.json(vendor);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("vendors:write");
    const body = await req.json();
    const parsed = VendorPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.vendor.findFirst({ where: { id: params.id, orgId, deletedAt: null } });
    if (!existing) return errorResponse("Vendor not found", 404);

    const vendor = await prisma.vendor.update({ where: { id: params.id }, data: parsed.data });
    await logAudit({ orgId, userId, action: "UPDATE", entityType: "Vendor", entityId: params.id, before: existing, after: vendor });
    return NextResponse.json(vendor);
  } catch (error: any) {
    if (error?.code === "P2002") return errorResponse("A vendor with this email already exists", 409);
    return handleRouteError(error);
  }
}

export async function DELETE(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("vendors:delete");
    const existing = await prisma.vendor.findFirst({ where: { id: params.id, orgId, deletedAt: null } });
    if (!existing) return errorResponse("Vendor not found", 404);

    await prisma.vendor.update({ where: { id: params.id }, data: { deletedAt: new Date() } });
    await logAudit({ orgId, userId, action: "DELETE", entityType: "Vendor", entityId: params.id, before: existing });
    return NextResponse.json({ success: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
