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
  test("a valid token yields its lane, org and location", async () => {
    const s = await seedLane();
    const ctx = await getLaneContext(req(`Bearer ${s.token}`));
    expect(ctx).toEqual({ laneId: s.lane.id, orgId: s.orgId, locationId: s.locationId });
    const after = await prisma.laneToken.findUnique({ where: { id: s.row.id } });
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

  test("a browser session is NOT accepted by getLaneContext", async () => {
    getServerSession.mockResolvedValue({ user: { id: "u", orgId: "o", role: "ADMIN" } });
    await seedLane();
    await expect(getLaneContext(req())).rejects.toBeInstanceOf(AuthError);
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
