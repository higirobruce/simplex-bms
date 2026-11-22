import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));
const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import { GET } from "@/app/api/[slug]/dashboard/stats/route";

const as = (userId: string, orgId: string) => getServerSession.mockResolvedValue({ user: { id: userId, orgId, role: "ADMIN" } });
const stats = async () => (await GET()).json();

beforeEach(() => {
  getServerSession.mockReset();
  vi.useFakeTimers({ toFake: ["Date"], now: new Date("2026-11-15T10:00:00Z") });
});
afterEach(() => vi.useRealTimers());

let n = 0;
async function till(s: { orgId: string; userId: string; locationId: string }) {
  const lane = await prisma.lane.create({ data: { code: `D${++n}`, name: "Lane", locationId: s.locationId, orgId: s.orgId } });
  const shift = await prisma.shift.create({
    data: { laneId: lane.id, orgId: s.orgId, openedById: s.userId, openedAt: new Date("2026-11-10T06:00:00Z"), openingFloat: "0.00" } as any,
  });
  return { laneId: lane.id, shiftId: shift.id };
}
let inv = 0;
async function sale(s: { orgId: string }, t: { laneId: string; shiftId: string }, totAmt: string, tenders: [string, string][], extra: Record<string, unknown> = {}) {
  return prisma.sale.create({
    data: {
      shiftId: t.shiftId, laneId: t.laneId, orgId: s.orgId, invcNo: ++inv, totAmt, totTaxblAmt: totAmt, totTaxAmt: "0.00",
      soldAt: new Date("2026-11-12T10:00:00Z"),
      tenders: { create: tenders.map(([pmtTyCd, amount], i) => ({ seq: i + 1, pmtTyCd: pmtTyCd as any, amount, orgId: s.orgId })) },
      ...extra,
    } as any,
  });
}
async function payment(s: { orgId: string }, amount: string, date: string) {
  const c = await prisma.customer.create({ data: { name: "C", orgId: s.orgId } as any });
  const invoice = await prisma.invoice.create({
    data: { invoiceNo: `INV-${++inv}`, customerId: c.id, orgId: s.orgId, subtotal: amount, total: amount, dueDate: new Date(date) } as any,
  });
  return prisma.payment.create({ data: { invoiceId: invoice.id, amount, method: "CASH", date: new Date(date) } as any });
}

describe("GET /api/[slug]/dashboard/stats revenueMTD", () => {
  test("counter takings come from Sale.totAmt, not from tenders", async () => {
    const s = await seedOrg(); const t = await till(s); as(s.userId, s.orgId);
    await sale(s, t, "700.00", [["CASH", "1000.00"]]); // 300 change
    await sale(s, t, "500.00", [["CASH", "200.00"], ["CARD", "300.00"]]);
    await sale(s, t, "999.00", [["CASH", "999.00"]], { voidedAt: new Date("2026-11-12T11:00:00Z") });
    expect((await stats()).revenueMTD.counter).toBe("1200.00");
  });

  test("invoice payments and counter takings are reported separately", async () => {
    const s = await seedOrg(); const t = await till(s); as(s.userId, s.orgId);
    await payment(s, "5000.00", "2026-11-05T10:00:00Z");
    await sale(s, t, "700.00", [["CASH", "700.00"]]);
    expect((await stats()).revenueMTD).toEqual({ invoices: "5000.00", counter: "700.00" });
  });

  test("another org's sales and payments are not counted", async () => {
    const s = await seedOrg(); const t = await till(s); as(s.userId, s.orgId);
    await sale({ orgId: s.otherOrgId }, t, "700.00", [["CASH", "700.00"]]);
    expect((await stats()).revenueMTD).toEqual({ invoices: "0.00", counter: "0.00" });
  });

  test("the month boundary is computed in the org's timezone", async () => {
    const s = await seedOrg(); const t = await till(s); as(s.userId, s.orgId);
    // Kigali is +2: November began 2026-10-31T22:00Z.
    await sale(s, t, "100.00", [["CASH", "100.00"]], { soldAt: new Date("2026-10-31T21:00:00Z") }); // still October
    await sale(s, t, "200.00", [["CASH", "200.00"]], { soldAt: new Date("2026-10-31T23:00:00Z") }); // November
    await payment(s, "30.00", "2026-10-31T21:00:00Z");
    await payment(s, "40.00", "2026-10-31T23:00:00Z");
    expect((await stats()).revenueMTD).toEqual({ invoices: "40.00", counter: "200.00" });
  });

  test("expenses keep their own boundary variable and do not shift zone", async () => {
    const s = await seedOrg(); as(s.userId, s.orgId);
    await prisma.org.update({ where: { id: s.orgId }, data: { timezone: "Pacific/Kiritimati" } }); // +14
    const serverStart = new Date(2026, 10, 1); // server-local, the boundary expenses always used
    const mk = (date: Date, amount: string) =>
      prisma.expense.create({ data: { category: "x", amount, date, orgId: s.orgId } });
    // An hour before the server-local boundary is November in Kiritimati but not for the server.
    await mk(new Date(serverStart.getTime() - 3600_000), "11.00");
    await mk(new Date(serverStart.getTime() + 3600_000), "22.00");
    expect((await stats()).expensesMTD).toBe(22);
  });
});
