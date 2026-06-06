import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError, errorResponse } from "@/lib/api";
import { ShopCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";
import bcrypt from "bcryptjs";

export async function GET(req: Request) {
  try {
    await requireSuperAdmin();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "50")));
    const search = url.searchParams.get("search") || "";
    const status = url.searchParams.get("status") || "";

    const where: any = { deletedAt: null };
    if (search) {
      where.OR = [
        { name: { contains: search, mode: "insensitive" } },
        { slug: { contains: search, mode: "insensitive" } },
      ];
    }
    if (status === "ACTIVE" || status === "SUSPENDED") where.status = status;

    const [data, total] = await Promise.all([
      prisma.org.findMany({
        where,
        select: {
          id: true,
          slug: true,
          name: true,
          logo: true,
          status: true,
          currency: true,
          timezone: true,
          createdAt: true,
          _count: { select: { users: true, products: true, invoices: true } },
        },
        orderBy: { createdAt: "desc" },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.org.count({ where }),
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
    const { userId } = await requireSuperAdmin();
    const body = await req.json();
    const parsed = ShopCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);
    const d = parsed.data;

    const existing = await prisma.org.findUnique({ where: { slug: d.slug } });
    if (existing) return errorResponse("That slug is already taken", 409);

    const passwordHash = await bcrypt.hash(d.adminPassword, 10);
    const org = await prisma.org.create({
      data: {
        slug: d.slug,
        name: d.name,
        currency: d.currency,
        timezone: d.timezone,
        users: {
          create: {
            email: d.adminEmail,
            name: d.adminName ?? d.adminEmail.split("@")[0],
            password: passwordHash,
            role: "ADMIN",
          },
        },
        locations: { create: { name: "Main Warehouse" } },
      },
      select: { id: true, slug: true, name: true, status: true, createdAt: true },
    });

    await logAudit({
      orgId: org.id,
      userId,
      action: "CREATE",
      entityType: "Org",
      entityId: org.id,
      after: org,
    });
    return NextResponse.json(org);
  } catch (error: any) {
    if (error?.code === "P2002") return errorResponse("A shop with that slug or admin email already exists", 409);
    return handleRouteError(error);
  }
}
