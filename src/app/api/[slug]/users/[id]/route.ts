import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { UserPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";
import bcrypt from "bcryptjs";

async function countAdmins(orgId: string, exceptUserId?: string) {
  return prisma.user.count({
    where: { orgId, role: "ADMIN", ...(exceptUserId ? { id: { not: exceptUserId } } : {}) },
  });
}

export async function PATCH(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("users:write");
    const body = await req.json();
    const parsed = UserPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.user.findFirst({ where: { id: params.id, orgId } });
    if (!existing) return errorResponse("User not found", 404);

    if (parsed.data.role && parsed.data.role !== "ADMIN" && existing.role === "ADMIN") {
      const remaining = await countAdmins(orgId, existing.id);
      if (remaining === 0) return errorResponse("Cannot demote the only administrator", 409);
    }

    const data: any = {};
    if (parsed.data.name !== undefined) data.name = parsed.data.name;
    if (parsed.data.role !== undefined) data.role = parsed.data.role;
    if (parsed.data.password) data.password = await bcrypt.hash(parsed.data.password, 10);

    const updated = await prisma.user.update({
      where: { id: params.id },
      data,
      select: { id: true, email: true, name: true, role: true, createdAt: true },
    });

    const beforeSafe = { id: existing.id, email: existing.email, name: existing.name, role: existing.role };
    const afterSafe = { ...updated, passwordChanged: !!parsed.data.password };
    await logAudit({ orgId, userId, action: "UPDATE", entityType: "User", entityId: params.id, before: beforeSafe, after: afterSafe });

    return NextResponse.json(updated);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function DELETE(_req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("users:write");
    if (params.id === userId) return errorResponse("You cannot delete your own account", 409);

    const existing = await prisma.user.findFirst({ where: { id: params.id, orgId } });
    if (!existing) return errorResponse("User not found", 404);

    if (existing.role === "ADMIN") {
      const remaining = await countAdmins(orgId, existing.id);
      if (remaining === 0) return errorResponse("Cannot delete the only administrator", 409);
    }

    await prisma.user.delete({ where: { id: params.id } });
    const beforeSafe = { id: existing.id, email: existing.email, name: existing.name, role: existing.role };
    await logAudit({ orgId, userId, action: "DELETE", entityType: "User", entityId: params.id, before: beforeSafe });

    return NextResponse.json({ success: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
