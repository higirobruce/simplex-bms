import { createHash, randomBytes } from "node:crypto";
import { prisma } from "@/lib/prisma";
import { AuthError } from "@/lib/api";

// Till (lane) authentication: a per-lane bearer token. Deliberately disjoint from the
// browser-session path in getTenantContext — this file never touches next-auth.
// SHA-256 (not a slow KDF) is correct here: the token is 256 bits of randomness.

export interface LaneContext {
  laneId: string;
  orgId: string;
  locationId: string;
}

export function hashLaneToken(token: string): string {
  return createHash("sha256").update(token).digest("hex");
}

export function mintLaneToken(): { token: string; tokenHash: string } {
  const token = randomBytes(32).toString("base64url");
  return { token, tokenHash: hashLaneToken(token) };
}

export async function getLaneContext(req: Request): Promise<LaneContext> {
  const header = req.headers.get("authorization") ?? "";
  const match = /^Bearer ([A-Za-z0-9_-]+)$/.exec(header.trim());
  if (!match) throw new AuthError();

  const row = await prisma.laneToken.findUnique({
    where: { tokenHash: hashLaneToken(match[1]) },
    include: { lane: { select: { id: true, orgId: true, locationId: true } } },
  });
  if (!row || row.revokedAt) throw new AuthError();

  await prisma.laneToken.update({ where: { id: row.id }, data: { lastUsedAt: new Date() } });

  // Tenant and location come from the Lane row, never from the request.
  return { laneId: row.lane.id, orgId: row.lane.orgId, locationId: row.lane.locationId };
}
