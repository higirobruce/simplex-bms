import { beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

// Route the app's prisma import to the guarded test client.
vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));

const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import { getLaneContext, mintLaneToken } from "@/lib/lane-auth";
import { AuthError } from "@/lib/api";

const req = (auth?: string) =>
  new Request("http://till.local/api/till/x", { headers: auth ? { authorization: auth } : {} });

async function seedLane() {
  const { orgId, locationId } = await seedOrg();
  const lane = await prisma.lane.create({ data: { code: "L1", name: "Lane 1", locationId, orgId } });
  const { token, tokenHash } = mintLaneToken();
  const row = await prisma.laneToken.create({ data: { laneId: lane.id, tokenHash, orgId } });
  return { orgId, locationId, lane, token, tokenHash, row };
}

beforeEach(() => getServerSession.mockReset());

describe("getLaneContext", () => {
  test("a valid token yields its lane, org and location; locationId comes from lane not first location", async () => {
    const { orgId, otherOrgId, locationId: firstLocationId } = await seedOrg();
    const secondLocation = await prisma.location.create({
      data: { name: "Second Store", code: "SECOND", orgId },
    });
    const lane = await prisma.lane.create({
      data: { code: "L1", name: "Lane 1", locationId: secondLocation.id, orgId },
    });
    const { token, tokenHash } = mintLaneToken();
    const row = await prisma.laneToken.create({
      data: { laneId: lane.id, tokenHash, orgId: otherOrgId }, // Use different org to test which one wins
    });
    const ctx = await getLaneContext(req(`Bearer ${token}`));
    expect(ctx).toEqual({ laneId: lane.id, orgId, locationId: secondLocation.id });
    expect(ctx.orgId).toBe(orgId); // Not otherOrgId — comes from the lane
    expect(ctx.locationId).toBe(secondLocation.id); // Not firstLocationId
    const after = await prisma.laneToken.findUnique({ where: { id: row.id } });
    expect(after?.lastUsedAt).toBeInstanceOf(Date);
  });

  test("a revoked token is refused", async () => {
    const s = await seedLane();
    await prisma.laneToken.update({ where: { id: s.row.id }, data: { revokedAt: new Date() } });
    await expect(getLaneContext(req(`Bearer ${s.token}`))).rejects.toBeInstanceOf(AuthError);
  });

  test("an unknown token is refused", async () => {
    await seedLane();
    await expect(getLaneContext(req(`Bearer ${mintLaneToken().token}`))).rejects.toBeInstanceOf(AuthError);
  });

  test("a missing Authorization header is refused", async () => {
    await seedLane();
    await expect(getLaneContext(req())).rejects.toBeInstanceOf(AuthError);
    await expect(getLaneContext(req("Basic abc"))).rejects.toBeInstanceOf(AuthError);
  });

  test("bearer scheme is case-insensitive", async () => {
    const s = await seedLane();
    const ctx = await getLaneContext(req(`bearer ${s.token}`));
    expect(ctx).toEqual({ laneId: s.lane.id, orgId: s.orgId, locationId: s.locationId });
  });

  test("a browser session is NOT accepted by getLaneContext even with both cookie and lane token", async () => {
    getServerSession.mockResolvedValue({ user: { id: "u", orgId: "o", role: "ADMIN" } });
    const s = await seedLane();
    const reqWithBoth = new Request("http://till.local/api/till/x", {
      headers: {
        authorization: `Bearer ${s.token}`,
        cookie: "next-auth.session-token=session-value",
      },
    });
    const ctx = await getLaneContext(reqWithBoth);
    // Context comes from lane auth, not from session
    expect(ctx).toEqual({ laneId: s.lane.id, orgId: s.orgId, locationId: s.locationId });
    // getServerSession should never be called — lane auth is disjoint
    expect(getServerSession).not.toHaveBeenCalled();
  });

  test("the token is never stored in plaintext", async () => {
    const s = await seedLane();
    expect(s.row.tokenHash).toBe(s.tokenHash);
    expect(s.tokenHash).not.toContain(s.token);
    const rows = await prisma.$queryRawUnsafe<Record<string, unknown>[]>(`select * from "LaneToken"`);
    expect(JSON.stringify(rows)).not.toContain(s.token);
  });

  test("minted tokens are unique, 32 bytes base64url, hashed with SHA-256 hex", () => {
    const a = mintLaneToken();
    expect(a.token).not.toBe(mintLaneToken().token);
    expect(Buffer.from(a.token, "base64url")).toHaveLength(32);
    expect(a.tokenHash).toMatch(/^[0-9a-f]{64}$/);
  });
});
