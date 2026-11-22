import { beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));
const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import { POST, GET, DELETE } from "@/app/api/[slug]/lanes/[id]/tokens/route";
import { getLaneContext } from "@/lib/lane-auth";
import { AuthError } from "@/lib/api";

const as = (userId: string, orgId: string, role = "ADMIN") =>
  getServerSession.mockResolvedValue({ user: { id: userId, orgId, role } });
const ctx = (id: string) => ({ params: { id } });
const url = "http://x.local/api/shop/lanes/l/tokens";
const post = (id: string, body?: unknown) =>
  POST(new Request(url, { method: "POST", body: body ? JSON.stringify(body) : undefined }), ctx(id));

async function seed() {
  const s = await seedOrg();
  const lane = await prisma.lane.create({ data: { code: "L1", name: "Lane 1", locationId: s.locationId, orgId: s.orgId } });
  as(s.userId, s.orgId);
  return { ...s, lane };
}

beforeEach(() => getServerSession.mockReset());

describe("lane token route", () => {
  test("an ADMIN can issue a token and sees its value exactly once", async () => {
    const s = await seed();
    const res = await post(s.lane.id, { label: "Till 1", orgId: s.otherOrgId });
    expect(res.status).toBe(201);
    const { id, token } = await res.json();
    expect(token).toBeTruthy();
    const row = await prisma.laneToken.findUniqueOrThrow({ where: { id } });
    expect(row.orgId).toBe(s.orgId); // from the Lane row, not the body
    expect(row.label).toBe("Till 1");
    expect(JSON.stringify(row)).not.toContain(token);
    expect(await getLaneContext(new Request(url, { headers: { authorization: `Bearer ${token}` } })))
      .toEqual({ laneId: s.lane.id, orgId: s.orgId, locationId: s.locationId });
    const audits = await prisma.auditLog.findMany({ where: { entityId: id } });
    expect(audits).toHaveLength(1);
    expect(JSON.stringify(audits)).not.toContain(token);
    expect(JSON.stringify(audits)).not.toContain(row.tokenHash);
  });

  test("listing tokens never returns a token value", async () => {
    const s = await seed();
    const { token } = await (await post(s.lane.id)).json();
    const res = await GET(new Request(url), ctx(s.lane.id));
    const text = await res.text();
    const { data } = JSON.parse(text);
    expect(data).toHaveLength(1);
    expect(Object.keys(data[0]).sort()).toEqual(["createdAt", "id", "label", "lastUsedAt", "revokedAt"]);
    const row = await prisma.laneToken.findFirstOrThrow();
    expect(text).not.toContain(token);
    expect(text).not.toContain(row.tokenHash);
  });

  test("a non-ADMIN cannot issue a token", async () => {
    const s = await seed();
    for (const role of ["MANAGER", "ACCOUNTANT", "VIEWER"]) {
      as(s.userId, s.orgId, role);
      expect((await post(s.lane.id)).status).toBe(403);
      expect((await GET(new Request(url), ctx(s.lane.id))).status).toBe(403);
    }
    expect(await prisma.laneToken.count()).toBe(0);
  });

  test("a revoked token stops authenticating", async () => {
    const s = await seed();
    const { id, token } = await (await post(s.lane.id)).json();
    const bearer = () => new Request(url, { headers: { authorization: `Bearer ${token}` } });
    await expect(getLaneContext(bearer())).resolves.toMatchObject({ laneId: s.lane.id });
    const res = await DELETE(new Request(`${url}?id=${id}`, { method: "DELETE" }), ctx(s.lane.id));
    expect(res.status).toBe(200);
    await expect(getLaneContext(bearer())).rejects.toBeInstanceOf(AuthError);
  });

  test("a lane in another org cannot be issued a token", async () => {
    const s = await seed();
    const foreignLane = await prisma.lane.create({
      data: { code: "F1", name: "Foreign", locationId: s.locationId, orgId: s.otherOrgId },
    });
    const res = await post(foreignLane.id);
    expect(res.status).toBe(404);
    expect(await prisma.laneToken.count()).toBe(0);
    expect((await GET(new Request(url), ctx(foreignLane.id))).status).toBe(404);
  });

  test("a token of another org's lane cannot be revoked", async () => {
    const s = await seed();
    const foreignLane = await prisma.lane.create({
      data: { code: "F1", name: "Foreign", locationId: s.locationId, orgId: s.otherOrgId },
    });
    const tok = await prisma.laneToken.create({ data: { laneId: foreignLane.id, tokenHash: "h".repeat(64), orgId: s.otherOrgId } });
    const res = await DELETE(new Request(`${url}?id=${tok.id}`, { method: "DELETE" }), ctx(foreignLane.id));
    expect(res.status).toBe(404);
    expect((await prisma.laneToken.findUniqueOrThrow({ where: { id: tok.id } })).revokedAt).toBeNull();
  });

  test("unauthenticated requests are 401", async () => {
    const s = await seed();
    getServerSession.mockResolvedValue(null);
    expect((await post(s.lane.id)).status).toBe(401);
  });
});
