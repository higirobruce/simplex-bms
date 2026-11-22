import { beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));
const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import { GET } from "@/app/api/[slug]/counter/[shiftId]/route";

const as = (userId: string, orgId: string) => getServerSession.mockResolvedValue({ user: { id: userId, orgId, role: "ADMIN" } });
const get = (shiftId: string) => GET(new Request("http://x.local/x"), { params: { shiftId } });
beforeEach(() => getServerSession.mockReset());

let n = 0;
let inv = 0;
async function seedShift(s: { orgId: string; userId: string; productId: string; locationId: string }) {
  const lane = await prisma.lane.create({ data: { code: `S${++n}`, name: `Lane ${n}`, locationId: s.locationId, orgId: s.orgId } });
  const shift = await prisma.shift.create({
    data: { laneId: lane.id, orgId: s.orgId, openedById: s.userId, openedAt: new Date("2026-11-22T06:00:00Z"), openingFloat: "0.00" },
  });
  const sale = await prisma.sale.create({
    data: {
      shiftId: shift.id, laneId: lane.id, orgId: s.orgId, invcNo: ++inv, totAmt: "1000.00", totTaxblAmt: "847.46", totTaxAmt: "152.54",
      lines: {
        create: [
          // qty*prc = 333.33 -> would round to 333; the till stored 334. Must come back 334.00.
          { itemSeq: 1, productId: s.productId, qty: "3.333", prc: "100.00", taxTyCd: "B", splyAmt: "334.00", taxblAmt: "283.05", orgId: s.orgId },
          { itemSeq: 2, productId: s.productId, qty: "1.000", prc: "666.00", taxTyCd: "B", splyAmt: null, taxblAmt: null, orgId: s.orgId },
        ],
      },
      tenders: { create: [{ seq: 1, pmtTyCd: "CASH", amount: "1000.00", orgId: s.orgId }] },
    } as any,
  });
  return { shift, sale };
}

describe("GET /api/[slug]/counter/[shiftId]", () => {
  test("a shift's sales come back with their lines and tenders", async () => {
    const s = await seedOrg(); as(s.userId, s.orgId);
    const { shift } = await seedShift(s);
    const res = await get(shift.id);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.sales).toHaveLength(1);
    expect(body.sales[0].totAmt).toBe("1000.00");
    expect(body.sales[0].lines.map((l: any) => l.name)).toEqual(["Test Product", "Test Product"]);
    expect(body.sales[0].tenders).toEqual([{ seq: 1, method: "CASH", amount: "1000.00" }]);
  });

  test("line amounts are the stored splyAmt, not a recomputation", async () => {
    const s = await seedOrg(); as(s.userId, s.orgId);
    const { shift } = await seedShift(s);
    const [l1, l2] = (await (await get(shift.id)).json()).sales[0].lines;
    expect(l1.splyAmt).toBe("334.00");
    expect(l1.taxblAmt).toBe("283.05");
    // Legacy row with no stored amounts: null, never derived from qty * prc.
    expect(l2.splyAmt).toBeNull();
    expect(l2.taxblAmt).toBeNull();
  });

  test("a shift from another org is 404", async () => {
    const s = await seedOrg(); as(s.userId, s.orgId);
    const mine = await seedShift(s);
    const other = await prisma.location.create({ data: { name: "O", code: "O", orgId: s.otherOrgId } });
    const p = await prisma.product.create({ data: { sku: `O-${Math.random()}`, name: "Theirs", unitPrice: "1.00", orgId: s.otherOrgId } });
    const theirs = await seedShift({ orgId: s.otherOrgId, userId: s.userId, productId: p.id, locationId: other.id });
    const res = await get(theirs.shift.id);
    expect(res.status).toBe(404);
    expect((await get("nonexistent")).status).toBe(404);
    expect((await get(mine.shift.id)).status).toBe(200);
  });

  test("the response is labelled as a back-office record, not a fiscal copy", async () => {
    const s = await seedOrg(); as(s.userId, s.orgId);
    const { shift } = await seedShift(s);
    const body = await (await get(shift.id)).json();
    expect(body.record.fiscalCopy).toBe(false);
    expect(body.record.kind).toBe("back-office");
    expect(body.record.note).toMatch(/not the fiscal receipt/i);
  });

  test("an unauthenticated caller is refused", async () => {
    getServerSession.mockResolvedValue(null);
    expect((await get("x")).status).toBe(401);
  });
});
