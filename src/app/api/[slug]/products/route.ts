import { prisma } from "@/lib/prisma";
import { requirePermission, getTenantContext, handleRouteError, errorResponse } from "@/lib/api";
import { ProductCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "20")));
    const search = url.searchParams.get("search") || "";
    const sortBy = url.searchParams.get("sortBy") || "createdAt";
    const sortOrder = (url.searchParams.get("sortOrder") || "desc") as "asc" | "desc";

    const where: any = { orgId, deletedAt: null };
    if (search) {
      where.OR = [
        { name: { contains: search, mode: "insensitive" } },
        { sku: { contains: search, mode: "insensitive" } },
        { category: { contains: search, mode: "insensitive" } },
      ];
    }

    const [data, total] = await Promise.all([
      prisma.product.findMany({
        where,
        include: { stockLevels: true },
        orderBy: { [sortBy]: sortOrder },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.product.count({ where }),
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
    const { orgId, userId } = await requirePermission("products:write");
    const body = await req.json();
    const parsed = ProductCreateSchema.safeParse(body);
    if (!parsed.success) {
      return errorResponse(parsed.error.issues[0].message, 422);
    }

    const product = await prisma.product.create({
      data: { orgId, ...parsed.data },
    });

    const location = await prisma.location.findFirst({ where: { orgId } });
    if (location) {
      await prisma.stockLevel.create({
        data: { productId: product.id, locationId: location.id, qty: 0 },
      });
    }

    await logAudit({ orgId, userId, action: "CREATE", entityType: "Product", entityId: product.id, after: product });

    return NextResponse.json(product);
  } catch (error: any) {
    if (error?.code === "P2002") {
      return errorResponse("A product with this SKU already exists", 409);
    }
    return handleRouteError(error);
  }
}
