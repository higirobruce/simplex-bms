import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { ProductPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId } = await getTenantContext();
    const product = await prisma.product.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: {
        stockLevels: { include: { location: true } },
        movements: { orderBy: { createdAt: "desc" }, take: 20 },
      },
    });
    if (!product) return errorResponse("Product not found", 404);
    return NextResponse.json(product);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("products:write");
    const body = await req.json();
    const parsed = ProductPatchSchema.safeParse(body);
    if (!parsed.success) {
      return errorResponse(parsed.error.issues[0].message, 422);
    }

    const existing = await prisma.product.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
    });
    if (!existing) return errorResponse("Product not found", 404);

    const product = await prisma.product.update({
      where: { id: params.id },
      data: parsed.data,
    });

    await logAudit({ orgId, userId, action: "UPDATE", entityType: "Product", entityId: params.id, before: existing, after: product });

    return NextResponse.json(product);
  } catch (error: any) {
    if (error?.code === "P2002") return errorResponse("Duplicate value conflict", 409);
    return handleRouteError(error);
  }
}

export async function DELETE(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("products:delete");
    const existing = await prisma.product.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
    });
    if (!existing) return errorResponse("Product not found", 404);

    await prisma.product.update({
      where: { id: params.id },
      data: { deletedAt: new Date() },
    });

    await logAudit({ orgId, userId, action: "DELETE", entityType: "Product", entityId: params.id, before: existing });

    return NextResponse.json({ success: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
