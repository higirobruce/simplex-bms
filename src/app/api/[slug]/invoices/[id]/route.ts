import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { InvoicePatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId } = await getTenantContext();
    const invoice = await prisma.invoice.findFirst({
      where: { id: params.id, orgId, deletedAt: null },
      include: {
        customer: true,
        lineItems: { include: { product: { select: { name: true, sku: true } } } },
        payments: { orderBy: { date: "desc" } },
      },
    });
    if (!invoice) return errorResponse("Invoice not found", 404);
    return NextResponse.json(invoice);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("invoices:write");
    const body = await req.json();
    const parsed = InvoicePatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.invoice.findFirst({ where: { id: params.id, orgId, deletedAt: null } });
    if (!existing) return errorResponse("Invoice not found", 404);

    const data: any = { ...parsed.data };
    if (data.dueDate) data.dueDate = new Date(data.dueDate);

    const invoice = await prisma.invoice.update({ where: { id: params.id }, data });
    await logAudit({ orgId, userId, action: "UPDATE", entityType: "Invoice", entityId: params.id, before: existing, after: invoice });
    return NextResponse.json(invoice);
  } catch (error) {
    return handleRouteError(error);
  }
}
