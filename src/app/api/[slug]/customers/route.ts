import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { CustomerCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "20")));
    const search = url.searchParams.get("search") || "";
    const status = url.searchParams.get("status") || undefined;

    const where: any = { orgId, deletedAt: null };
    if (search) {
      where.OR = [
        { name: { contains: search, mode: "insensitive" } },
        { email: { contains: search, mode: "insensitive" } },
        { phone: { contains: search, mode: "insensitive" } },
      ];
    }
    if (status) where.status = status;

    const [data, total] = await Promise.all([
      prisma.customer.findMany({
        where,
        include: { _count: { select: { invoices: true } } },
        orderBy: { createdAt: "desc" },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.customer.count({ where }),
    ]);

    return NextResponse.json({
      data,
      meta: { total, page, limit, pages: Math.ceil(total / limit) },
    });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("customers:write");
    const body = await req.json();
    const parsed = CustomerCreateSchema.safeParse(body);
    if (!parsed.success) {
      return errorResponse(parsed.error.issues[0].message, 422);
    }

    const customer = await prisma.customer.create({
      data: { orgId, ...parsed.data },
    });

    await logAudit({ orgId, userId, action: "CREATE", entityType: "Customer", entityId: customer.id, after: customer });

    return NextResponse.json(customer);
  } catch (error: any) {
    if (error?.code === "P2002") return errorResponse("A customer with this email already exists", 409);
    return handleRouteError(error);
  }
}
