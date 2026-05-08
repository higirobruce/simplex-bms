import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { CustomerPatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId } = await getTenantContext();
    const customer = await prisma.customer.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: { invoices: { orderBy: { createdAt: "desc" }, take: 10 }, _count: { select: { invoices: true } } },
    });
    if (!customer) return errorResponse("Customer not found", 404);
    return NextResponse.json(customer);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("customers:write");
    const body = await req.json();
    const parsed = CustomerPatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.customer.findFirst({ where: { id: params.id, orgId, deletedAt: null } });
    if (!existing) return errorResponse("Customer not found", 404);

    const customer = await prisma.customer.update({ where: { id: params.id }, data: parsed.data });
    await logAudit({ orgId, userId, action: "UPDATE", entityType: "Customer", entityId: params.id, before: existing, after: customer });
    return NextResponse.json(customer);
  } catch (error: any) {
    if (error?.code === "P2002") return errorResponse("A customer with this email already exists", 409);
    return handleRouteError(error);
  }
}

export async function DELETE(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("customers:delete");
    const existing = await prisma.customer.findFirst({ where: { id: params.id, orgId, deletedAt: null } });
    if (!existing) return errorResponse("Customer not found", 404);

    const invoiceCount = await prisma.invoice.count({ where: { customerId: params.id, orgId, status: { notIn: ["VOID"] }, deletedAt: null } });
    if (invoiceCount > 0) return errorResponse("Cannot delete customer with active invoices", 409);

    await prisma.customer.update({ where: { id: params.id }, data: { deletedAt: new Date() } });
    await logAudit({ orgId, userId, action: "DELETE", entityType: "Customer", entityId: params.id, before: existing });
    return NextResponse.json({ success: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
