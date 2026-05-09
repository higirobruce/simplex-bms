import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { UserCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";
import bcrypt from "bcryptjs";

export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "50")));
    const search = url.searchParams.get("search") || "";

    const where: any = { orgId };
    if (search) {
      where.OR = [
        { name: { contains: search, mode: "insensitive" } },
        { email: { contains: search, mode: "insensitive" } },
      ];
    }

    const [data, total] = await Promise.all([
      prisma.user.findMany({
        where,
        select: { id: true, email: true, name: true, role: true, createdAt: true },
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

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("users:write");
    const body = await req.json();
    const parsed = UserCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const passwordHash = await bcrypt.hash(parsed.data.password, 10);
    const user = await prisma.user.create({
      data: {
        orgId,
        email: parsed.data.email,
        name: parsed.data.name ?? null,
        role: parsed.data.role,
        password: passwordHash,
      },
      select: { id: true, email: true, name: true, role: true, createdAt: true },
    });

    await logAudit({ orgId, userId, action: "CREATE", entityType: "User", entityId: user.id, after: user });
    return NextResponse.json(user);
  } catch (error: any) {
    if (error?.code === "P2002") return errorResponse("A user with this email already exists", 409);
    return handleRouteError(error);
  }
}
