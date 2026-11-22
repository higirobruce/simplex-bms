import { beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));
const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import { GET } from "@/app/api/[slug]/counter/route";

const as = (userId: string, orgId: string) => getServerSession.mockResolvedValue({ user: { id: userId, orgId, role: "ADMIN" } });
const get = (day?: string) => GET(new Request(`http://x.local/api/shop/counter${day ? `?day=${day}` : ""}`));
beforeEach(() => getServerSession.mockReset());

let n = 0;
async function lane(s: { orgId: string; locationId: string }) {
  return prisma.lane.create({ data: { code: `L${++n}`, name: `Lane ${n}`, locationId: s.locationId, orgId: s.orgId } });
}
async function shift(s: { orgId: string; userId: string }, laneId: string, extra: Record<string, unknown> = {}) {
  return prisma.shift.create({
    data: { laneId, orgId: s.orgId, openedById: s.userId, openedAt: new Date("2026-11-22T06:00:00Z"), openingFloat: "20000.00", ...extra } as any,
  });
}
let inv = 0;
async function sale(s: { orgId: string }, shiftId: string, laneId: string, totAmt: string, tenders: [string, string][], extra: Record<string, unknown> = {}) {
  return prisma.sale.create({
    data: {
      shiftId, laneId, orgId: s.orgId, invcNo: ++inv, totAmt, totTaxblAmt: totAmt, totTaxAmt: "0.00",
      tenders: { create: tenders.map(([pmtTyCd, amount], i) => ({ seq: i + 1, pmtTyCd: pmtTyCd as any, amount, orgId: s.orgId })) },
      ...extra,
    } as any,
  });
}

describe("GET /api/[slug]/counter", () => {
  test("shifts are grouped by the day they OPENED, in the org's timezone", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    await shift(s, l.id, { openedAt: new Date("2026-11-22T06:00:00Z") });
    await shift(s, l.id, { openedAt: new Date("2026-11-21T06:00:00Z") });
    const body = await (await get("2026-11-22")).json();
    expect(body.day).toBe("2026-11-22");
    expect(body.shifts).toHaveLength(1);
  });

  test("a shift opening 01:30 Kigali is not filed under the previous day", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    await shift(s, l.id, { openedAt: new Date("2026-11-21T23:30:00Z") }); // 01:30 on the 22nd in Kigali
    expect((await (await get("2026-11-21")).json()).shifts).toHaveLength(0);
    expect((await (await get("2026-11-22")).json()).shifts).toHaveLength(1);
  });

  test("the method split sums to takings, with change shown as a deduction", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    const sh = await shift(s, l.id, { closedAt: new Date("2026-11-22T14:00:00Z"), saleCount: 2, gross: "1200.00", declaredCash: "20800.00" });
    await sale(s, sh.id, l.id, "700.00", [["CASH", "1000.00"]]);
    await sale(s, sh.id, l.id, "500.00", [["CASH", "200.00"], ["CARD", "300.00"]]);
    const [row] = (await (await get("2026-11-22")).json()).shifts;
    const sum = row.methods.reduce((a: number, m: any) => a + Number(m.tendered), 0);
    expect(row.takings).toBe("1200.00");
    expect(row.changeGiven).toBe("300.00");
    expect(sum - Number(row.changeGiven)).toBe(Number(row.takings));
    expect(row.state).toBe("complete");
    // expected = 20000 + 1200 cash - 300 change = 20900; declared 20800 -> short 100
    expect(row.variance).toEqual({ expected: "20900.00", declared: "20800.00", diff: "-100.00" });
  });

  test("credit sales appear as their own line and are not hidden in the total", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    const sh = await shift(s, l.id, { closedAt: new Date("2026-11-22T14:00:00Z"), saleCount: 2, gross: "900.00", declaredCash: "20400.00" });
    await sale(s, sh.id, l.id, "400.00", [["CASH", "400.00"]]);
    await sale(s, sh.id, l.id, "500.00", [["CREDIT", "500.00"]]);
    const body = await (await get("2026-11-22")).json();
    expect(body.shifts[0].methods).toContainEqual({ method: "CREDIT", tendered: "500.00" });
    expect(body.total).toBe("900.00");
  });

  test("the shop total is marked unqualified when any shift is not complete", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    const ok = await shift(s, l.id, { closedAt: new Date("2026-11-22T14:00:00Z"), saleCount: 1, gross: "100.00", declaredCash: "20100.00" });
    await sale(s, ok.id, l.id, "100.00", [["CASH", "100.00"]]);
    expect((await (await get("2026-11-22")).json()).totalQualified).toBe(true);
    const bad = await shift(s, l.id, { closedAt: new Date("2026-11-22T15:00:00Z"), saleCount: 3, gross: "900.00", declaredCash: "1.00" });
    await sale(s, bad.id, l.id, "100.00", [["CASH", "100.00"]]);
    const body = await (await get("2026-11-22")).json();
    expect(body.totalQualified).toBe(false);
    const badRow = body.shifts.find((r: any) => r.id === bad.id);
    expect(badRow.state).toBe("short_count");
    expect(badRow.variance).toBeNull();
  });

  test("a closed shift with no till totals is unknown and shows no variance", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    await shift(s, l.id, { closedAt: new Date("2026-11-22T14:00:00Z"), declaredCash: "20000.00" });
    const [row] = (await (await get("2026-11-22")).json()).shifts;
    expect(row.state).toBe("unknown");
    expect(row.variance).toBeNull();
  });

  test("voided sales are excluded and a lane with no tokens does not crash", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    const sh = await shift(s, l.id, { closedAt: new Date("2026-11-22T14:00:00Z"), saleCount: 1, gross: "100.00", declaredCash: "20100.00" });
    await sale(s, sh.id, l.id, "100.00", [["CASH", "100.00"]]);
    await sale(s, sh.id, l.id, "999.00", [["CASH", "999.00"]], { voidedAt: new Date() });
    const [row] = (await (await get("2026-11-22")).json()).shifts;
    expect(row.state).toBe("complete");
    expect(row.lastReachedAt).toBeNull();
  });

  test("the cashier name resolves, even for a deactivated cashier", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    const c = await prisma.user.create({
      data: { email: `c-${Math.random()}@c.invalid`, password: "!x", name: "M. Uwase", role: "CASHIER", pinHash: null, orgId: s.orgId },
    });
    await shift(s, l.id, { openedById: c.id });
    expect((await (await get("2026-11-22")).json()).shifts[0].cashier.name).toBe("M. Uwase");
  });

  test("money is returned as strings, never numbers", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    const sh = await shift(s, l.id, { closedAt: new Date("2026-11-22T14:00:00Z"), saleCount: 1, gross: "100.00", declaredCash: "20100.00" });
    await sale(s, sh.id, l.id, "100.00", [["CASH", "100.00"]]);
    const body = await (await get("2026-11-22")).json();
    const r = body.shifts[0];
    for (const v of [body.total, r.openingFloat, r.takings, r.changeGiven, r.variance.expected, r.variance.declared, r.variance.diff, r.methods[0].tendered]) {
      expect(typeof v).toBe("string");
    }
  });

  test("another org's shifts never appear", async () => {
    const s = await seedOrg(); const l = await lane(s); as(s.userId, s.orgId);
    const otherLoc = await prisma.location.create({ data: { name: "X", code: "X", orgId: s.otherOrgId } });
    const ol = await prisma.lane.create({ data: { code: "OL", name: "OL", locationId: otherLoc.id, orgId: s.otherOrgId } });
    await shift({ orgId: s.otherOrgId, userId: s.userId }, ol.id);
    await shift(s, l.id);
    expect((await (await get("2026-11-22")).json()).shifts).toHaveLength(1);
  });

  test("an unreal day is a 400", async () => {
    const s = await seedOrg(); as(s.userId, s.orgId);
    expect((await get("2026-02-30")).status).toBe(400);
  });

  test("an unauthenticated request is refused", async () => {
    getServerSession.mockResolvedValue(null);
    expect((await get("2026-11-22")).status).toBe(401);
  });
});
