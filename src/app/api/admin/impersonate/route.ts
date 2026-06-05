import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError, errorResponse, ACTING_ORG_COOKIE } from "@/lib/api";
import { NextResponse } from "next/server";

// Enter a shop: set the acting-org cookie so the super admin's shop-scoped
// requests resolve to this org (with ADMIN permissions).
export async function POST(req: Request) {
  try {
    await requireSuperAdmin();
    const { orgId } = await req.json();
    if (!orgId) return errorResponse("orgId is required", 400);

    const org = await prisma.org.findUnique({
      where: { id: orgId },
      select: { id: true, slug: true, name: true },
    });
    if (!org) return errorResponse("Shop not found", 404);

    const res = NextResponse.json({ slug: org.slug, name: org.name });
    res.cookies.set(ACTING_ORG_COOKIE, org.id, {
      httpOnly: true,
      sameSite: "lax",
      path: "/",
    });
    return res;
  } catch (error) {
    return handleRouteError(error);
  }
}

// Exit the shop: clear the acting-org cookie.
export async function DELETE() {
  try {
    await requireSuperAdmin();
    const res = NextResponse.json({ ok: true });
    res.cookies.delete(ACTING_ORG_COOKIE);
    return res;
  } catch (error) {
    return handleRouteError(error);
  }
}
