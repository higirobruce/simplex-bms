import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { ProductImportSchema, ProductImportRowSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

// Bulk product import. Validates each row independently and reports a per-row
// outcome (created / skipped-duplicate / error) rather than failing the whole
// file on one bad row.
export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("products:write");
    const body = await req.json();
    const parsed = ProductImportSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const results: { row: number; status: "created" | "skipped" | "error"; message?: string }[] = [];
    let created = 0;
    let skipped = 0;

    for (let i = 0; i < parsed.data.rows.length; i++) {
      const rowNo = i + 1;
      const row = ProductImportRowSchema.safeParse(parsed.data.rows[i]);
      if (!row.success) {
        results.push({ row: rowNo, status: "error", message: row.error.issues[0].message });
        continue;
      }
      try {
        await prisma.product.create({ data: { orgId, ...row.data } });
        created++;
        results.push({ row: rowNo, status: "created" });
      } catch (e: any) {
        if (e?.code === "P2002") {
          skipped++;
          results.push({ row: rowNo, status: "skipped", message: `SKU "${row.data.sku}" already exists` });
        } else {
          results.push({ row: rowNo, status: "error", message: "Could not import" });
        }
      }
    }

    const errors = results.filter((r) => r.status === "error").length;
    if (created > 0) {
      await logAudit({
        orgId,
        userId,
        action: "CREATE",
        entityType: "ProductImport",
        entityId: orgId,
        after: { created, skipped, errors, total: results.length },
      });
    }

    return NextResponse.json({ created, skipped, errors, total: results.length, results });
  } catch (error) {
    return handleRouteError(error);
  }
}
