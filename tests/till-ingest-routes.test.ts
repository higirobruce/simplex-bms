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

  const CLOSE = { closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "100.00" };

  test("a close carrying saleCount and gross stores both", async () => {
    const s = await seed();
    await post(s.token, open(s));
    const res = await post(s.token, open(s, { ...CLOSE, saleCount: 12, gross: "3380.30" }));
    expect(res.status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.saleCount).toBe(12);
    expect(row.gross?.toFixed(2)).toBe("3380.30");
  });

  test("a zero saleCount and gross are stored as zero, not null", async () => {
    const s = await seed();
    const res = await post(s.token, open(s, { ...CLOSE, saleCount: 0, gross: "0.00" }));
    expect(res.status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.saleCount).toBe(0);
    expect(row.gross?.toFixed(2)).toBe("0.00");
  });

  test("a close without them stores nulls and is still accepted", async () => {
    const s = await seed();
    await post(s.token, open(s));
    expect((await post(s.token, open(s, CLOSE))).status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.closedAt).not.toBeNull();
    expect(row.saleCount).toBeNull();
    expect(row.gross).toBeNull();
  });

  test("they are refused on an OPEN payload with 422", async () => {
    const s = await seed();
    expect((await post(s.token, open(s, { saleCount: 3 }))).status).toBe(422);
    expect((await post(s.token, open(s, { gross: "1.00" }))).status).toBe(422);
    expect(await prisma.shift.count()).toBe(0);
  });

  test("a replayed close writes them on the existing row", async () => {
    const s = await seed();
    await post(s.token, open(s));
    await post(s.token, open(s, CLOSE)); // first close arrived without them
    expect((await post(s.token, open(s, { ...CLOSE, saleCount: 4, gross: "55.50" }))).status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.saleCount).toBe(4);
    expect(row.gross?.toFixed(2)).toBe("55.50");
  });

  test("a replayed close without them does not erase stored totals", async () => {
    const s = await seed();
    await post(s.token, open(s, { ...CLOSE, saleCount: 4, gross: "55.50" }));
    expect((await post(s.token, open(s, CLOSE))).status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.saleCount).toBe(4);
    expect(row.gross?.toFixed(2)).toBe("55.50");
  });

  test("a non-integer saleCount or a bad money string is 422", async () => {
    const s = await seed();
    for (const extra of [
      { saleCount: 1.5 }, { saleCount: -1 }, { saleCount: "3" }, { saleCount: 2147483648 }, { saleCount: 1e300 },
      { gross: 12.5 }, { gross: "12.505" }, { gross: "-1.00" }, { gross: "abc" },
    ]) {
      expect((await post(s.token, open(s, { ...CLOSE, ...extra }))).status).toBe(422);
    }
    expect(await prisma.shift.count()).toBe(0);
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
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.closedAt?.toISOString()).toBe("2026-11-22T17:00:00.000Z");
    expect(row.declaredCash?.toFixed(2)).toBe("5.00");
  });

  test("a shift id owned by a different org cannot be overwritten", async () => {
    const s = await seed();
    const otherLane = await prisma.lane.create({ data: { code: "O1", name: "O1", locationId: s.locationId, orgId: s.otherOrgId } });
    const theirs = await s.mk(s.otherOrgId, "Theirs");
    await prisma.shift.create({
      data: { id: SID, laneId: otherLane.id, orgId: s.otherOrgId, openedById: theirs.id, openingFloat: "1.00" },
    });
    const res = await post(s.token, open(s, { closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "9.00" }));
    expect(res.status).toBe(422);
    expect((await prisma.shift.findUniqueOrThrow({ where: { id: SID } })).closedAt).toBeNull();
  });

  test("openedById mismatching the existing shift is refused", async () => {
    const s = await seed();
    await post(s.token, open(s));
    const other = await s.mk(s.orgId, "Bob");
    const res = await post(s.token, open(s, { openedById: other.id, closedAt: "2026-11-22T17:00:00.000Z", declaredCash: "9.00" }));
    expect(res.status).toBe(422);
    expect((await prisma.shift.findUniqueOrThrow({ where: { id: SID } })).closedAt).toBeNull();
  });

  test("dates need an explicit instant: lenient forms are 422, Z and offsets accepted", async () => {
    const s = await seed();
    for (const openedAt of ["2026", "1", "2026-11-22T10:00:00", "2026-11-22"]) {
      expect((await post(s.token, open(s, { openedAt }))).status).toBe(422);
    }
    expect(await prisma.shift.count()).toBe(0);
    expect((await post(s.token, open(s, { openedAt: "2026-11-22T10:00:00+02:00" }))).status).toBe(200);
    const row = await prisma.shift.findUniqueOrThrow({ where: { id: SID } });
    expect(row.openedAt.toISOString()).toBe("2026-11-22T08:00:00.000Z");
    expect((await post(s.token, open(s, { openedAt: "2026-11-22T08:00:00.000Z" }))).status).toBe(200);
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
