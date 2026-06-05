import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError, errorResponse, ACTING_ORG_COOKIE } from "@/lib/api";
import { logAudit } from "@/lib/audit";
import { cookies } from "next/headers";
import { NextResponse } from "next/server";

// Enter a shop: set the acting-org cookie so the super admin's shop-scoped
// requests resolve to this org (with ADMIN permissions).
export async function POST(req: Request) {
  try {
    const { userId } = await requireSuperAdmin();
    const { orgId } = await req.json();
    if (!orgId) return errorResponse("orgId is required", 400);

    const org = await prisma.org.findUnique({
      where: { id: orgId },
      select: { id: true, slug: true, name: true },
    });
    if (!org) return errorResponse("Shop not found", 404);

    // Record who stepped into which shop — impersonation is the most privileged
    // action in the console, so it belongs in the shop's audit trail.
    await logAudit({
      orgId: org.id,
      userId,
      action: "IMPERSONATE_START",
      entityType: "Org",
      entityId: org.id,
    });

    const res = NextResponse.json({ slug: org.slug, name: org.name });
    res.cookies.set(ACTING_ORG_COOKIE, org.id, {
      httpOnly: true,
      sameSite: "lax",
      secure: process.env.NODE_ENV === "production",
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
    const { userId } = await requireSuperAdmin();

    // Log the exit against the shop being left (read before clearing).
    // Best-effort: never let a stale cookie (shop since deleted) block exiting.
    const actingOrgId = cookies().get(ACTING_ORG_COOKIE)?.value;
    if (actingOrgId) {
      try {
        await logAudit({
          orgId: actingOrgId,
          userId,
          action: "IMPERSONATE_STOP",
          entityType: "Org",
          entityId: actingOrgId,
        });
      } catch {
        // org no longer exists — nothing to log against; continue clearing.
      }
    }

    const res = NextResponse.json({ ok: true });
    res.cookies.delete(ACTING_ORG_COOKIE);
    return res;
  } catch (error) {
    return handleRouteError(error);
  }
}
