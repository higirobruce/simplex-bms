import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { BulkImportSchema, StockImportRowSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";
import type { ImportRowResult } from "@/lib/import";

// Opening-stock import: each row (sku, location, qty) resolves a product and a
// location within the org, then records a RECEIPT movement and adds the qty to
// the stock level. Re-running adds more stock (no dedupe).
export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("stock:adjust");
    const body = await req.json();
    const parsed = BulkImportSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    // Resolve lookups once for the org.
    const [products, locations] = await Promise.all([
      prisma.product.findMany({ where: { orgId, deletedAt: null }, select: { id: true, sku: true } }),
      prisma.location.findMany({ where: { orgId }, select: { id: true, name: true, code: true } }),
    ]);
    const bySku = new Map(products.map((p) => [p.sku.toLowerCase(), p.id]));
    const byLoc = new Map<string, string>();
    for (const l of locations) {
      byLoc.set(l.name.toLowerCase(), l.id);
      if (l.code) byLoc.set(l.code.toLowerCase(), l.id);
    }

    const results: ImportRowResult[] = [];
    let created = 0;

    for (let i = 0; i < parsed.data.rows.length; i++) {
      const rowNo = i + 1;
      const row = StockImportRowSchema.safeParse(parsed.data.rows[i]);
      if (!row.success) {
        results.push({ row: rowNo, status: "error", message: row.error.issues[0].message });
        continue;
      }
      const productId = bySku.get(row.data.sku.toLowerCase());
      if (!productId) {
        results.push({ row: rowNo, status: "error", message: `Unknown SKU "${row.data.sku}"` });
        continue;
      }
      const locationId = byLoc.get(row.data.location.toLowerCase());
      if (!locationId) {
        results.push({ row: rowNo, status: "error", message: `Unknown location "${row.data.location}"` });
        continue;
      }
      try {
        await prisma.$transaction(async (tx) => {
          const current = await tx.stockLevel.findUnique({
            where: { productId_locationId: { productId, locationId } },
          });
          await tx.stockLevel.upsert({
            where: { productId_locationId: { productId, locationId } },
            update: { qty: (current?.qty ?? 0) + row.data.qty },
            create: { productId, locationId, qty: row.data.qty },
          });
          await tx.stockMovement.create({
            data: {
              productId,
              toLocationId: locationId,
              qty: row.data.qty,
              type: "RECEIPT",
              reason: "Opening stock import",
              orgId,
            },
          });
        });
        created++;
        results.push({ row: rowNo, status: "created" });
      } catch {
        results.push({ row: rowNo, status: "error", message: "Could not import" });
      }
    }

    const errors = results.filter((r) => r.status === "error").length;
    if (created > 0) {
      await logAudit({
        orgId,
        userId,
        action: "CREATE",
        entityType: "StockImport",
        entityId: orgId,
        after: { created, errors, total: results.length },
      });
    }
    return NextResponse.json({ created, skipped: 0, errors, total: results.length, results });
  } catch (error) {
    return handleRouteError(error);
  }
}
