import { prisma } from "@/lib/prisma";
import { requirePermission, getTenantContext, handleRouteError, errorResponse } from "@/lib/api";
import { InvoiceCreateSchema } from "@/lib/validations";
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
        { invoiceNo: { contains: search, mode: "insensitive" } },
        { customer: { name: { contains: search, mode: "insensitive" } } },
      ];
    }
    if (status) where.status = status;

    const [data, total] = await Promise.all([
      prisma.invoice.findMany({
        where,
        include: { customer: { select: { name: true } }, _count: { select: { lineItems: true } } },
        orderBy: { createdAt: "desc" },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.invoice.count({ where }),
    ]);

    return NextResponse.json({ data, meta: { total, page, limit, pages: Math.ceil(total / limit) } });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("invoices:write");
    const body = await req.json();
    const parsed = InvoiceCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const { customerId, lineItems, dueDate, notes } = parsed.data;

    const customer = await prisma.customer.findFirst({ where: { id: customerId, orgId, deletedAt: null } });
    if (!customer) return errorResponse("Customer not found", 404);

    const subtotal = lineItems.reduce((sum, item) => sum + item.qty * item.unitPrice, 0);

    const invoice = await prisma.$transaction(async (tx) => {
      const seq = await tx.$queryRaw<[{ lastSeq: number }]>`
        WITH max_existing AS (
          SELECT COALESCE(MAX(CAST(SUBSTRING("invoiceNo" FROM 5) AS INTEGER)), 0) AS n
          FROM "Invoice"
          WHERE "orgId" = ${orgId} AND "invoiceNo" ~ '^INV-[0-9]+$'
        )
        INSERT INTO "InvoiceSequence" ("orgId", "lastSeq")
        SELECT ${orgId}, n + 1 FROM max_existing
        ON CONFLICT ("orgId") DO UPDATE
          SET "lastSeq" = GREATEST(
            "InvoiceSequence"."lastSeq" + 1,
            (SELECT n + 1 FROM max_existing)
          )
        RETURNING "lastSeq"
      `;
      const invoiceNo = `INV-${String(seq[0].lastSeq).padStart(4, "0")}`;

      return tx.invoice.create({
        data: {
          orgId, invoiceNo, customerId, subtotal, total: subtotal,
          dueDate: new Date(dueDate), notes: notes || null,
          lineItems: {
            create: lineItems.map((item) => ({
              productId: item.productId || null,
              description: item.description || null,
              qty: item.qty, unitPrice: item.unitPrice,
              amount: item.qty * item.unitPrice,
            })),
          },
        },
        include: { lineItems: true, customer: true },
      });
    });

    await logAudit({ orgId, userId, action: "CREATE", entityType: "Invoice", entityId: invoice.id, after: invoice });

    return NextResponse.json(invoice);
  } catch (error) {
    return handleRouteError(error);
  }
}
