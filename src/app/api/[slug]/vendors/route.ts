import { prisma } from "@/lib/prisma";
import { getTenantContext, requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { VendorCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(url.searchParams.get("limit") || "20")));
    const search = url.searchParams.get("search") || "";

    const where: any = { orgId, deletedAt: null };
    if (search) {
      where.OR = [
        { name: { contains: search, mode: "insensitive" } },
        { email: { contains: search, mode: "insensitive" } },
      ];
    }

    const [data, total] = await Promise.all([
      prisma.vendor.findMany({ where, orderBy: { createdAt: "desc" }, skip: (page - 1) * limit, take: limit }),
      prisma.vendor.count({ where }),
    ]);

    return NextResponse.json({ data, meta: { total, page, limit, pages: Math.ceil(total / limit) } });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("vendors:write");
    const body = await req.json();
    const parsed = VendorCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const vendor = await prisma.vendor.create({ data: { orgId, ...parsed.data } });
    await logAudit({ orgId, userId, action: "CREATE", entityType: "Vendor", entityId: vendor.id, after: vendor });
    return NextResponse.json(vendor);
  } catch (error: any) {
    if (error?.code === "P2002") return errorResponse("A vendor with this email already exists", 409);
    return handleRouteError(error);
  }
}
