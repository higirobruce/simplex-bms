import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { ExpensePatchSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId } = await getTenantContext();
    const expense = await prisma.expense.findFirst({ where: { id: params.id, orgId } });
    if (!expense) return errorResponse("Expense not found", 404);
    return NextResponse.json(expense);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("expenses:write");
    const body = await req.json();
    const parsed = ExpensePatchSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const existing = await prisma.expense.findFirst({ where: { id: params.id, orgId } });
    if (!existing) return errorResponse("Expense not found", 404);

    const data: any = { ...parsed.data };
    if (data.date) data.date = new Date(data.date);

    const expense = await prisma.expense.update({ where: { id: params.id }, data });
    await logAudit({ orgId, userId, action: "UPDATE", entityType: "Expense", entityId: params.id, before: existing, after: expense });
    return NextResponse.json(expense);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function DELETE(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("expenses:delete");
    const existing = await prisma.expense.findFirst({ where: { id: params.id, orgId } });
    if (!existing) return errorResponse("Expense not found", 404);

    await prisma.expense.delete({ where: { id: params.id } });
    await logAudit({ orgId, userId, action: "DELETE", entityType: "Expense", entityId: params.id, before: existing });
    return NextResponse.json({ success: true });
  } catch (error) {
    return handleRouteError(error);
  }
}
