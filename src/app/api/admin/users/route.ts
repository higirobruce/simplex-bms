import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError } from "@/lib/api";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    await requireSuperAdmin();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "50")));
    const search = url.searchParams.get("search") || "";
    const orgId = url.searchParams.get("orgId") || "";

    // Platform-wide view: every shop user (super admins excluded).
    const where: any = { isSuperAdmin: false };
    if (orgId) where.orgId = orgId;
    if (search) {
      where.OR = [
        { name: { contains: search, mode: "insensitive" } },
        { email: { contains: search, mode: "insensitive" } },
      ];
    }

    const [data, total] = await Promise.all([
      prisma.user.findMany({
        where,
        select: {
          id: true,
          email: true,
          name: true,
          role: true,
          createdAt: true,
          org: { select: { id: true, slug: true, name: true, status: true } },
        },
        orderBy: { createdAt: "desc" },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.user.count({ where }),
    ]);

    return NextResponse.json({
      data,
      meta: { total, page, limit, pages: Math.ceil(total / limit) },
    });
  } catch (error) {
    return handleRouteError(error);
  }
}
