import { prisma } from "@/lib/prisma";
import { getTenantContext, handleRouteError } from "@/lib/api";
import { NextResponse } from "next/server";

// Paginated stock-level rows (one row per product × location), filtered by an
// optional location subtree and a product name/SKU search. Summary totals are
// org-wide (independent of the filters), matching the page's summary cards.
export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "25")));
    const search = url.searchParams.get("search") || "";
    const locationId = url.searchParams.get("locationId") || "";

    // Resolve the selected location's subtree (self + all descendants).
    let locationIds: string[] | undefined;
    if (locationId) {
      const locs = await prisma.location.findMany({
        where: { orgId },
        select: { id: true, parentId: true },
      });
      const childrenOf = new Map<string, string[]>();
      for (const l of locs) {
        if (!l.parentId) continue;
        const arr = childrenOf.get(l.parentId) ?? [];
        arr.push(l.id);
        childrenOf.set(l.parentId, arr);
      }
      const ids: string[] = [];
      const stack = [locationId];
      while (stack.length) {
        const id = stack.pop()!;
        ids.push(id);
        for (const c of childrenOf.get(id) ?? []) stack.push(c);
      }
      locationIds = ids;
    }

    const productWhere: any = { orgId, deletedAt: null };
    if (search) {
      productWhere.OR = [
        { name: { contains: search, mode: "insensitive" } },
        { sku: { contains: search, mode: "insensitive" } },
      ];
    }
    const where: any = { product: productWhere };
    if (locationIds) where.locationId = { in: locationIds };

    const [levels, total] = await Promise.all([
      prisma.stockLevel.findMany({
        where,
        select: {
          qty: true,
          locationId: true,
          location: { select: { name: true } },
          product: { select: { id: true, sku: true, name: true, reorderLevel: true } },
        },
        orderBy: [{ product: { name: "asc" } }, { locationId: "asc" }],
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.stockLevel.count({ where }),
    ]);

    // Org-wide summary (not affected by the location/search filters).
    const totalsWhere = { product: { orgId, deletedAt: null } };
    const [agg, lowRows] = await Promise.all([
      prisma.stockLevel.aggregate({ where: totalsWhere, _sum: { qty: true } }),
      prisma.$queryRaw<{ count: number }[]>`
        SELECT count(*)::int AS count
        FROM "StockLevel" sl
        JOIN "Product" p ON p.id = sl."productId"
        WHERE p."orgId" = ${orgId} AND p."deletedAt" IS NULL AND sl.qty <= p."reorderLevel"
      `,
    ]);

    const data = levels.map((l) => ({
      productId: l.product.id,
      sku: l.product.sku,
      name: l.product.name,
      locationId: l.locationId,
      locationName: l.location?.name ?? "Unknown",
      qty: l.qty,
      reorderLevel: l.product.reorderLevel,
    }));

    return NextResponse.json({
      data,
      meta: { total, page, limit, pages: Math.ceil(total / limit) },
      summary: {
        totalUnits: agg._sum.qty ?? 0,
        lowStockCount: lowRows[0]?.count ?? 0,
      },
    });
  } catch (error) {
    return handleRouteError(error);
  }
}
