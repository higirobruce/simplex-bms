import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { mintLaneToken } from "@/lib/lane-auth";
import { logAudit } from "@/lib/audit";
import { NextResponse } from "next/server";

// Lane device tokens. ADMIN only (org:write). The lane is always looked up
// scoped by the caller's session org, and a token's orgId is copied from that
// Lane row, never from the body or URL slug. The plaintext token is returned
// by POST only; it is not stored, logged or audited (only its hash is persisted,
// and the hash is kept out of the audit log too).

const findLane = (orgId: string, id: string) =>
  prisma.lane.findFirst({ where: { id, orgId }, select: { id: true, orgId: true, code: true } });

export async function POST(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("org:write");
    const lane = await findLane(orgId, params.id);
    if (!lane) return errorResponse("Lane not found", 404);

    let label: string | null = null;
    try {
      const body = await req.json();
      if (typeof body?.label === "string" && body.label.trim()) label = body.label.trim().slice(0, 100);
    } catch {
      // no/invalid body: label is optional
    }

    const { token, tokenHash } = mintLaneToken();
    const row = await prisma.laneToken.create({
      data: { laneId: lane.id, tokenHash, label, orgId: lane.orgId },
      select: { id: true },
    });

    await logAudit({
      orgId: lane.orgId,
      userId,
      action: "CREATE",
      entityType: "LaneToken",
      entityId: row.id,
      after: { laneId: lane.id, label },
    });

    return NextResponse.json({ id: row.id, token }, { status: 201 });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function GET(_req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId } = await requirePermission("org:write");
    const lane = await findLane(orgId, params.id);
    if (!lane) return errorResponse("Lane not found", 404);

    const data = await prisma.laneToken.findMany({
      where: { laneId: lane.id, orgId },
      select: { id: true, label: true, createdAt: true, lastUsedAt: true, revokedAt: true },
      orderBy: { createdAt: "desc" },
    });
    return NextResponse.json({ data });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function DELETE(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId, userId } = await requirePermission("org:write");
    const lane = await findLane(orgId, params.id);
    if (!lane) return errorResponse("Lane not found", 404);

    const tokenId = new URL(req.url).searchParams.get("id");
    if (!tokenId) return errorResponse("id is required", 400);

    const existing = await prisma.laneToken.findFirst({
      where: { id: tokenId, laneId: lane.id, orgId },
      select: { id: true, revokedAt: true },
    });
    if (!existing) return errorResponse("Token not found", 404);
    if (existing.revokedAt) return NextResponse.json({ id: existing.id, revokedAt: existing.revokedAt });

    const revokedAt = new Date();
    await prisma.laneToken.update({ where: { id: existing.id }, data: { revokedAt } });
    await logAudit({
      orgId,
      userId,
      action: "UPDATE",
      entityType: "LaneToken",
      entityId: existing.id,
      before: { revokedAt: null },
      after: { revokedAt: revokedAt.toISOString(), laneId: lane.id },
    });
    return NextResponse.json({ id: existing.id, revokedAt });
  } catch (error) {
    return handleRouteError(error);
  }
}
