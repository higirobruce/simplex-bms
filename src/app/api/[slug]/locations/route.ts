import { prisma } from "@/lib/prisma";
import { getTenantId, handleRouteError, errorResponse } from "@/lib/api";
import { NextResponse } from "next/server";

export async function GET() {
  try {
    const orgId = await getTenantId();
    const locations = await prisma.location.findMany({
      where: { orgId },
      orderBy: { createdAt: "asc" },
    });
    return NextResponse.json(locations);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function POST(req: Request) {
  try {
    const orgId = await getTenantId();
    const { name, address } = await req.json();
    if (!name) return errorResponse("Location name is required");

    const location = await prisma.location.create({
      data: { orgId, name, address: address || null },
    });
    return NextResponse.json(location);
  } catch (error) {
    return handleRouteError(error);
  }
}
