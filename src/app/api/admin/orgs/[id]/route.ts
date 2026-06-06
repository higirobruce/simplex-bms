import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError, errorResponse } from "@/lib/api";
import { ShopPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(_req: Request, { params }: { params: { id: string } }) {
  try {
    await requireSuperAdmin();
    const org = await prisma.org.findUnique({
      where: { id: params.id },
      select: {
        id: true,
        slug: true,
        name: true,
        logo: true,
        status: true,
        currency: true,
        timezone: true,
        createdAt: true,
        _count: { select: { users: true, products: true, invoices: true, customers: true } },
      },
    });
    if (!org) return errorResponse("Shop not found", 404);
    return NextResponse.json(org);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request, { params }: { params: { id: string } }) {
  try {
    const { userId } = await requireSuperAdmin();
    const body = await req.json();
    const parsed = ShopPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const before = await prisma.org.findUnique({
      where: { id: params.id },
      select: { id: true, slug: true, name: true, logo: true, status: true, currency: true, timezone: true },
    });
    if (!before) return errorResponse("Shop not found", 404);

    const after = await prisma.org.update({
      where: { id: params.id },
      data: parsed.data,
      select: { id: true, slug: true, name: true, logo: true, status: true, currency: true, timezone: true, createdAt: true },
    });

    await logAudit({
      orgId: params.id,
      userId,
      action: before.status !== after.status ? "STATUS_CHANGE" : "UPDATE",
      entityType: "Org",
      entityId: params.id,
      before,
      after,
    });
    return NextResponse.json(after);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function DELETE(_req: Request, { params }: { params: { id: string } }) {
  try {
    const { userId } = await requireSuperAdmin();
    const before = await prisma.org.findUnique({
      where: { id: params.id },
      select: { id: true, slug: true, name: true, status: true, deletedAt: true },
    });
    if (!before || before.deletedAt) return errorResponse("Shop not found", 404);

    // Soft delete — preserves all shop data (and this audit trail, which would
    // otherwise cascade away with a hard delete) and keeps it recoverable.
    await prisma.org.update({
      where: { id: params.id },
      data: { deletedAt: new Date() },
    });

    await logAudit({
      orgId: params.id,
      userId,
      action: "DELETE",
      entityType: "Org",
      entityId: params.id,
      before,
    });
    return NextResponse.json({ ok: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
