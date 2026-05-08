import { prisma } from "@/lib/prisma";
import { getTenantContext, handleRouteError, errorResponse } from "@/lib/api";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    const { orgId, role } = await getTenantContext();

    // Only ADMIN and MANAGER can view audit logs
    if (role !== "ADMIN" && role !== "MANAGER") {
      return errorResponse("Forbidden", 403);
    }

    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "25")));
    const entityType = url.searchParams.get("entityType") || undefined;

    const where: any = { orgId };
    if (entityType) where.entityType = entityType;

    const [data, total] = await Promise.all([
      prisma.auditLog.findMany({
        where,
        orderBy: { createdAt: "desc" },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.auditLog.count({ where }),
    ]);

    return NextResponse.json({
      data,
      meta: {
        total,
        page,
        limit,
        pages: Math.ceil(total / limit),
      },
    });
  } catch (error) {
    return handleRouteError(error);
  }
}
