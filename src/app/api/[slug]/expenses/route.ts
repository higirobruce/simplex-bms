import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { ExpenseCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "20")));
    const search = url.searchParams.get("search") || "";
    const category = url.searchParams.get("category") || undefined;

    const where: any = { orgId };
    if (search) {
      where.OR = [
        { category: { contains: search, mode: "insensitive" } },
        { description: { contains: search, mode: "insensitive" } },
      ];
    }
    if (category) where.category = category;

    const [data, total] = await Promise.all([
      prisma.expense.findMany({ where, orderBy: { date: "desc" }, skip: (page - 1) * limit, take: limit }),
      prisma.expense.count({ where }),
    ]);

    return NextResponse.json({ data, meta: { total, page, limit, pages: Math.ceil(total / limit) } });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("expenses:write");
    const body = await req.json();
    const parsed = ExpenseCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const expense = await prisma.expense.create({
      data: {
        orgId, category: parsed.data.category, amount: parsed.data.amount,
        date: parsed.data.date ? new Date(parsed.data.date) : new Date(),
        description: parsed.data.description || null,
      },
    });

    await logAudit({ orgId, userId, action: "CREATE", entityType: "Expense", entityId: expense.id, after: expense });

    return NextResponse.json(expense);
  } catch (error) {
    return handleRouteError(error);
  }
}
