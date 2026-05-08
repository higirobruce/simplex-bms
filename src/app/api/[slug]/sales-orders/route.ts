import { prisma } from "@/lib/prisma";
import {
  getTenantContext,
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { SalesOrderCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { nextDocNo } from "@/lib/doc-sequence";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(
      100,
      Math.max(1, parseInt(url.searchParams.get("limit") || "20"))
    );
    const search = url.searchParams.get("search") || "";
    const status = url.searchParams.get("status") || undefined;

    const where: any = { orgId, deletedAt: null };
    if (search) {
      where.OR = [
        { orderNo: { contains: search, mode: "insensitive" } },
        { customer: { name: { contains: search, mode: "insensitive" } } },
      ];
    }
    if (status) where.status = status;

    const [data, total] = await Promise.all([
      prisma.salesOrder.findMany({
        where,
        include: {
          customer: { select: { name: true } },
          _count: { select: { lines: true, deliveryNotes: true } },
        },
        orderBy: { createdAt: "desc" },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.salesOrder.count({ where }),
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
    const { orgId, userId } = await requirePermission("sales:write");
    const body = await req.json();
    const parsed = SalesOrderCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const { customerId, expectedDate, notes, lines } = parsed.data;

    const customer = await prisma.customer.findFirst({
      where: { id: customerId, orgId, deletedAt: null },
    });
    if (!customer) return errorResponse("Customer not found", 404);

    const productIds = Array.from(new Set(lines.map((l) => l.productId)));
    const products = await prisma.product.findMany({
      where: { id: { in: productIds }, orgId, deletedAt: null },
      select: { id: true },
    });
    if (products.length !== productIds.length) {
      return errorResponse("One or more products not found", 404);
    }

    const subtotal = lines.reduce((s, l) => s + l.qty * l.unitPrice, 0);

    const order = await prisma.$transaction(async (tx) => {
      const orderNo = await nextDocNo(tx, orgId, "SO", "SalesOrder", "SO");
      return tx.salesOrder.create({
        data: {
          orgId,
          orderNo,
          customerId,
          expectedDate: expectedDate ? new Date(expectedDate) : null,
          notes: notes || null,
          subtotal,
          total: subtotal,
          createdById: userId,
          lines: {
            create: lines.map((l) => ({
              productId: l.productId,
              qty: l.qty,
              unitPrice: l.unitPrice,
              amount: l.qty * l.unitPrice,
            })),
          },
        },
        include: { lines: true, customer: true },
      });
    });

    await logAudit({
      orgId,
      userId,
      action: "CREATE",
      entityType: "SalesOrder",
      entityId: order.id,
      after: order,
    });

    return NextResponse.json(order);
  } catch (error) {
    return handleRouteError(error);
  }
}
