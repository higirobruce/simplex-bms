import { beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));
const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import { POST } from "@/app/api/till/shift/route";
import { GET } from "@/app/api/till/cashiers/route";
import { mintLaneToken } from "@/lib/lane-auth";

const url = "http://x.local/api/till/shift";
const SID = "a".repeat(32);

async function seed() {
  const s = await seedOrg();
  const lane = await prisma.lane.create({ data: { code: "L1", name: "Lane 1", locationId: s.locationId, orgId: s.orgId } });
  const { token, tokenHash } = mintLaneToken();
  const lt = await prisma.laneToken.create({ data: { laneId: lane.id, tokenHash, orgId: s.orgId } });
  const mk = (orgId: string, name: string, pinHash: string | null = "$argon2id$fake") =>
    prisma.user.create({
      data: { email: `${name}-${Math.random()}@c.invalid`, password: "!x", name, role: "CASHIER", pinHash, orgId },
    });
  const cashier = await mk(s.orgId, "Amy");
  return { ...s, lane, token, tokenId: lt.id, cashier, mk };
}
const post = (token: string | null, body: unknown) =>
  POST(new Request(url, {
    method: "POST",
    headers: token ? { authorization: `Bearer ${token}` } : {},
    body: typeof body === "string" ? body : JSON.stringify(body),
  }));
const open = (s: { cashier: { id: string } }, extra: object = {}) => ({
  id: SID, openedById: s.cashier.id, openedAt: "2026-11-22T08:00:00.000Z", openingFloat: "20000.00", ...extra,
});

beforeEach(() => getServerSession.mockReset());

describe("POST /api/till/shift", () => {
  test("a shift is created with the lane and org from the token, not the body", async () => {
    const s = await seed();
    const res = await post(s.token, open(s, { orgId: s.otherOrgId, laneId: "nope" }));
    expect(res.status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.orgId).toBe(s.orgId);
    expect(row.laneId).toBe(s.lane.id);
    expect(row.openingFloat.toString()).toBe("20000");
    expect(await prisma.shift.count()).toBe(1);
  });

  test("re-posting the same shift id is idempotent", async () => {
    const s = await seed();
    expect((await post(s.token, open(s))).status).toBe(200);
    expect((await post(s.token, open(s))).status).toBe(200);
    expect(await prisma.shift.count()).toBe(1);
  });

  test("a replayed open does not reopen a closed shift", async () => {
    const s = await seed();
    await post(s.token, open(s));
    await post(s.token, open(s, { closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "1.50" }));
    await post(s.token, open(s));
    expect((await prisma.shift.findUniqueOrThrow({ where: { id: SID } })).closedAt).not.toBeNull();
  });

  test("a close updates closedAt and declaredCash on the existing row", async () => {
    const s = await seed();
    await post(s.token, open(s));
    const res = await post(s.token, open(s, { closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "123456.70" }));
    expect(res.status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.closedAt?.toISOString()).toBe("2026-11-22T17:00:00.000Z");
    expect(row.declaredCash?.toFixed(2)).toBe("123456.70");
    expect(await prisma.shift.count()).toBe(1);
  });

  test("openedById must be a CASHIER in this org", async () => {
    const s = await seed();
    for (const id of [s.userId, "does-not-exist"]) {
      const res = await post(s.token, open(s, { openedById: id }));
      expect(res.status).toBe(422);
    }
    expect(await prisma.shift.count()).toBe(0);
  });

  test("an inactive cashier's shift is refused with a non-retryable status", async () => {
    const s = await seed();
    const inactive = await s.mk(s.orgId, "Gone", null);
    const res = await post(s.token, open(s, { openedById: inactive.id }));
    expect(res.status).toBe(422);
    expect(await prisma.shift.count()).toBe(0);
  });

  test("a close for a cashier deactivated after opening is still accepted", async () => {
    const s = await seed();
    await post(s.token, open(s));
    await prisma.user.update({ where: { id: s.cashier.id }, data: { pinHash: null } });
    const res = await post(s.token, open(s, { closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "5.00" }));
    expect(res.status).toBe(200);
  });

  test("a shift naming a cashier from another org is refused", async () => {
    const s = await seed();
    const foreign = await s.mk(s.otherOrgId, "Zed");
    expect((await post(s.token, open(s, { openedById: foreign.id }))).status).toBe(422);
    expect(await prisma.shift.count()).toBe(0);
  });

  test("a shift id owned by another lane cannot be overwritten", async () => {
    const s = await seed();
    const l2 = await prisma.lane.create({ data: { code: "L2", name: "L2", locationId: s.locationId, orgId: s.orgId } });
    await prisma.shift.create({
      data: { id: SID, laneId: l2.id, orgId: s.orgId, openedById: s.cashier.id, openingFloat: "1.00" },
    });
    const res = await post(s.token, open(s, { closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "9.00" }));
    expect(res.status).toBe(422);
    expect((await prisma.shift.findUniqueOrThrow({ where: { id: SID } })).closedAt).toBeNull();
  });

  test("malformed bodies are 422", async () => {
    const s = await seed();
    const bad = [
      "not json", "[]", {}, open(s, { id: "short" }), open(s, { openingFloat: 20000 }),
      open(s, { openingFloat: "1e3" }), open(s, { openingFloat: "-1.00" }), open(s, { openedAt: "yesterday" }),
      open(s, { closedAt: "2026-11-22T17:00:00.000Z" }), open(s, { declaredCash: "1.00" }),
      open(s, { closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "1.234" }),
    ];
    for (const b of bad) expect((await post(s.token, b)).status).toBe(422);
    expect(await prisma.shift.count()).toBe(0);
  });

  test("a database fault is 5xx (retryable), not 422", async () => {
    const s = await seed();
    const spy = vi.spyOn(prisma.user, "findFirst").mockRejectedValueOnce(new Error("connection refused"));
    const res = await post(s.token, open(s));
    spy.mockRestore();
    expect(res.status).toBeGreaterThanOrEqual(500);
    expect(await prisma.shift.count()).toBe(0);
  });

  test("a missing or revoked token is 401 and writes nothing", async () => {
    const s = await seed();
    expect((await post(null, open(s))).status).toBe(401);
    expect((await post("bogus", open(s))).status).toBe(401);
    await prisma.laneToken.update({ where: { id: s.tokenId }, data: { revokedAt: new Date() } });
    expect((await post(s.token, open(s))).status).toBe(401);
    expect(await prisma.shift.count()).toBe(0);
  });
});

describe("GET /api/till/cashiers", () => {
  const get = (token: string | null) =>
    GET(new Request("http://x.local/api/till/cashiers", { headers: token ? { authorization: `Bearer ${token}` } : {} }));

  test("returns this org's cashiers with pinHash and derived active", async () => {
    const s = await seed();
    await s.mk(s.orgId, "Off", null);
    await s.mk(s.otherOrgId, "Foreign");
    const res = await get(s.token);
    expect(res.status).toBe(200);
    const list = await res.json();
    expect(list).toHaveLength(2);
    const amy = list.find((c: any) => c.name === "Amy");
    expect(amy).toEqual({ id: s.cashier.id, name: "Amy", pinHash: "$argon2id$fake", active: true });
    expect(list.find((c: any) => c.name === "Off")).toMatchObject({ pinHash: null, active: false });
    expect(list.some((c: any) => c.name === "Foreign")).toBe(false);
  });

  test("refuses a browser session and requires a lane token", async () => {
    const s = await seed();
    getServerSession.mockResolvedValue({ user: { id: s.userId, orgId: s.orgId, role: "ADMIN" } });
    const res = await get(null);
    expect(res.status).toBe(401);
    expect(await res.text()).not.toContain("argon2");
    expect(getServerSession).not.toHaveBeenCalled();
  });

  test("a revoked token stops working immediately", async () => {
    const s = await seed();
    expect((await get(s.token)).status).toBe(200);
    await prisma.laneToken.update({ where: { id: s.tokenId }, data: { revokedAt: new Date() } });
    expect((await get(s.token)).status).toBe(401);
  });
});
