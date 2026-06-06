import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { BulkImportSchema, VendorImportRowSchema } from "@/lib/validations";
import { runRowImport } from "@/lib/import";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("vendors:write");
    const body = await req.json();
    const parsed = BulkImportSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const summary = await runRowImport(
      parsed.data.rows,
      VendorImportRowSchema,
      (data) => prisma.vendor.create({ data: { orgId, ...data } }).then(() => undefined),
      (data) => `Email "${data.email}" already exists`
    );

    if (summary.created > 0) {
      await logAudit({
        orgId,
        userId,
        action: "CREATE",
        entityType: "VendorImport",
        entityId: orgId,
        after: { created: summary.created, skipped: summary.skipped, errors: summary.errors, total: summary.total },
      });
    }
    return NextResponse.json(summary);
  } catch (error) {
    return handleRouteError(error);
  }
}
