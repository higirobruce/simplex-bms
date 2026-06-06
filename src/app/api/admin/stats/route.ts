import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError } from "@/lib/api";
import { NextResponse } from "next/server";

export async function GET() {
  try {
    await requireSuperAdmin();

    const [totalShops, activeShops, suspendedShops, totalUsers, recentShops] =
      await Promise.all([
        prisma.org.count({ where: { deletedAt: null } }),
        prisma.org.count({ where: { deletedAt: null, status: "ACTIVE" } }),
        prisma.org.count({ where: { deletedAt: null, status: "SUSPENDED" } }),
        prisma.user.count({ where: { isSuperAdmin: false, org: { deletedAt: null } } }),
        prisma.org.findMany({
          where: { deletedAt: null },
          orderBy: { createdAt: "desc" },
          take: 5,
          select: {
            id: true,
            slug: true,
            name: true,
            status: true,
            createdAt: true,
            _count: { select: { users: true } },
          },
        }),
      ]);

    return NextResponse.json({
      totalShops,
      activeShops,
      suspendedShops,
      totalUsers,
      recentShops,
    });
  } catch (error) {
    return handleRouteError(error);
  }
}
