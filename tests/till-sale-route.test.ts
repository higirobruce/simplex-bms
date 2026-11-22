import { beforeEach, describe, expect, test, vi } from "vitest";
import { prisma, seedOrg } from "./helpers/db";

vi.mock("@/lib/prisma", async () => ({ prisma: (await import("./helpers/db")).prisma }));
const getServerSession = vi.hoisted(() => vi.fn());
vi.mock("next-auth", () => ({ getServerSession }));

import { POST } from "@/app/api/till/sale/route";
import { mintLaneToken } from "@/lib/lane-auth";

const SALE = "b".repeat(32);
const SHIFT = "a".repeat(32);

async function seed() {
  const s = await seedOrg();
  // The lane sits on a SECOND location, so a route that picked the org's first location would be caught.
  const laneLoc = await prisma.location.create({ data: { name: "Shop Floor", code: "FLOOR", orgId: s.orgId } });
  const lane = await prisma.lane.create({ data: { code: "L1", name: "Lane 1", locationId: laneLoc.id, orgId: s.orgId } });
  const { token, tokenHash } = mintLaneToken();
  await prisma.laneToken.create({ data: { laneId: lane.id, tokenHash, orgId: s.orgId } });
  const cashier = await prisma.user.create({
    data: { email: `c-${Math.random()}@c.invalid`, password: "!x", name: "Amy", role: "CASHIER", pinHash: "$argon2id$fake", orgId: s.orgId },
  });
  await prisma.shift.create({ data: { id: SHIFT, laneId: lane.id, orgId: s.orgId, openedById: cashier.id, openingFloat: "0.00" } });
  return { ...s, lane, token, laneLocationId: laneLoc.id };
}
const post = (token: string | null, body: unknown) =>
  POST(new Request("http://x.local/api/till/sale", {
    method: "POST",
    headers: token ? { authorization: `Bearer ${token}` } : {},
    body: JSON.stringify(body),
  }));
const sale = (s: { productId: string }, extra: object = {}) => ({
  saleId: SALE, shiftId: SHIFT, invcNo: 5, totAmt: "2400.00", totTaxblAmt: "2033.90", totTaxAmt: "366.10",
  soldAt: "2026-11-20T09:30:00.000Z",
  lines: [{ itemSeq: 1, productId: s.productId, qty: "2.000", prc: "1200.00", taxTyCd: "B" }],
  tenders: [{ pmtTyCd: "CASH", amount: "2400.00" }],
  ...extra,
});
const counts = async () => ({
  sale: await prisma.sale.count(), line: await prisma.saleLine.count(), tender: await prisma.tender.count(),
  mv: await prisma.stockMovement.count({ where: { type: "SALE" } }),
});

beforeEach(() => getServerSession.mockReset());

describe("POST /api/till/sale", () => {
  test("a sale lands with its lines, tenders and one SALE movement per line", async () => {
    const s = await seed();
    const p2 = await prisma.product.create({ data: { sku: "P2", name: "P2", unitPrice: "5.00", orgId: s.orgId } });
    const res = await post(s.token, sale(s, {
      lines: [
        { itemSeq: 1, productId: s.productId, qty: "2.000", prc: "1200.00", taxTyCd: "B" },
        { itemSeq: 2, productId: p2.id, qty: "1.500", prc: "5.00", taxTyCd: "A" },
      ],
      tenders: [{ pmtTyCd: "CASH", amount: "1000.00" }, { pmtTyCd: "MOBILE_MONEY", amount: "1407.50" }],
    }));
    expect(res.status).toBe(200);
    expect(await counts()).toEqual({ sale: 1, line: 2, tender: 2, mv: 2 });
    const mv = await prisma.stockMovement.findMany({ where: { type: "SALE" }, orderBy: { qty: "asc" } });
    expect(mv.map((m) => m.referenceId)).toEqual([SALE, SALE]);
    expect(mv.map((m) => m.qty.toFixed(3))).toEqual(["1.500", "2.000"]);
    expect((await prisma.sale.findUniqueOrThrow({ where: { id: SALE } })).orgId).toBe(s.orgId);
  });

  test("the movement's fromLocationId is the lane's location", async () => {
    const s = await seed();
    const other = await prisma.location.create({ data: { name: "Other", code: "OTH", orgId: s.orgId } });
    await post(s.token, sale(s, { fromLocationId: other.id, locationId: other.id }));
    const mv = await prisma.stockMovement.findFirstOrThrow({ where: { type: "SALE" } });
    expect(mv.fromLocationId).toBe(s.laneLocationId);
    expect(mv.fromLocationId).not.toBe(s.locationId);
    expect(mv.fromLocationId).not.toBe(other.id);
    expect(mv.toLocationId).toBeNull();
  });

  test("a sale decrements StockLevel by the summed qty at the lane's location; a re-POST does not", async () => {
    const s = await seed();
    await prisma.stockLevel.create({ data: { productId: s.productId, locationId: s.laneLocationId, qty: "10.000" } });
    await prisma.stockLevel.create({ data: { productId: s.productId, locationId: s.locationId, qty: "10.000" } });
    const body = sale(s, {
      lines: [
        { itemSeq: 1, productId: s.productId, qty: "1.250", prc: "10.00", taxTyCd: "B" },
        { itemSeq: 2, productId: s.productId, qty: "2.000", prc: "10.00", taxTyCd: "B" },
      ],
    });
    const level = async (loc: string) =>
      (await prisma.stockLevel.findUniqueOrThrow({ where: { productId_locationId: { productId: s.productId, locationId: loc } } })).qty.toFixed(3);
    expect((await post(s.token, body)).status).toBe(200);
    expect(await level(s.laneLocationId)).toBe("6.750");
    expect(await level(s.locationId)).toBe("10.000");
    expect((await post(s.token, body)).status).toBe(200);
    expect((await post(s.token, body)).status).toBe(200);
    expect(await level(s.laneLocationId)).toBe("6.750");
  });

  test("selling a product with no StockLevel row creates one at the lane's location, negative", async () => {
    const s = await seed();
    expect((await post(s.token, sale(s))).status).toBe(200);
    const lvl = await prisma.stockLevel.findFirstOrThrow({ where: { productId: s.productId } });
    expect(lvl.locationId).toBe(s.laneLocationId);
    expect(lvl.qty.toFixed(3)).toBe("-2.000");
    await post(s.token, sale(s));
    expect((await prisma.stockLevel.findFirstOrThrow({ where: { productId: s.productId } })).qty.toFixed(3)).toBe("-2.000");
  });

  // Field set copied from cloud_payload in till/crates/till-store/src/sale.rs, including the
  // extra till-only fields and the NESTED receipt, which is the only shape the till sends.
  test("the till's real payload shape is accepted and rcptNo is stored from receipt.rcptNo", async () => {
    const s = await seed();
    const golden = {
      saleId: SALE, shiftId: SHIFT, bhfId: "00", invcNo: 12, buyerTin: "123456789",
      totAmt: "2380.00", totTaxblAmt: "2016.95", totTaxAmt: "363.05", pmtTyCd: "CASH",
      createdAt: "2026-11-21T10:30:00Z", confirmedAt: "2026-11-21T10:30:02Z",
      lines: [{
        itemSeq: 1, itemCd: "SKU-X", itemClsCd: "5020230602", itemNm: "Thing", qty: "2.000", prc: "1200.00",
        splyAmt: "2400.00", dcRt: "0", dcAmt: "20.00", taxblAmt: "2380.00", taxAmt: "363.05", taxTyCd: "B",
      }],
      tenders: [{ seq: 1, pmtTyCd: "CASH", amount: "2380.00" }],
      receipt: {
        rcptNo: 42, intrlData: "INTRL", rcptSign: "ABCD1234EFGH5678", sdcId: "SDC010000001",
        mrcNo: "WIS00000001", vsdcRcptPbctDate: "20261121103000", totRcptNo: 7,
      },
    };
    const p = await prisma.product.findUniqueOrThrow({ where: { id: s.productId } });
    golden.lines[0].itemCd = p.sku;
    expect((await post(s.token, golden)).status).toBe(200);
    const row = await prisma.sale.findUniqueOrThrow({ where: { id: SALE } });
    expect(row.rcptNo).toBe(42);
    expect(row.buyerTin).toBe("123456789");
    expect(row.soldAt.toISOString()).toBe("2026-11-21T10:30:00.000Z");
    expect(row.invcNo).toBe(12);
  });

  test("a line discount round-trips exactly and the lines reconcile with the header", async () => {
    const s = await seed();
    const body = sale(s, {
      totAmt: "2380.00",
      lines: [{ itemSeq: 1, productId: s.productId, qty: "2.000", prc: "1200.00", dcRt: "0.83", dcAmt: "20.00", taxTyCd: "B" }],
    });
    expect((await post(s.token, body)).status).toBe(200);
    const l = await prisma.saleLine.findFirstOrThrow();
    expect(l.discountKind).toBe("AMOUNT");
    expect(l.discountValue?.toFixed(2)).toBe("20.00");
    const net = l.qty.mul(l.prc).minus(l.discountValue!);
    const head = await prisma.sale.findUniqueOrThrow({ where: { id: SALE } });
    expect(net.toFixed(2)).toBe(head.totAmt.toFixed(2));
  });

  test("an undiscounted line stores no discount", async () => {
    const s = await seed();
    await post(s.token, sale(s));
    const l = await prisma.saleLine.findFirstOrThrow();
    expect(l.discountKind).toBeNull();
    expect(l.discountValue).toBeNull();
  });

  test("a summed quantity that cannot fit its column is 422, not 500, and writes nothing", async () => {
    const s = await seed();
    const res = await post(s.token, sale(s, {
      lines: [
        { itemSeq: 1, productId: s.productId, qty: "999999999.000", prc: "1.00", taxTyCd: "B" },
        { itemSeq: 2, productId: s.productId, qty: "999999999.000", prc: "1.00", taxTyCd: "B" },
      ],
    }));
    expect(res.status).toBe(422);
    expect(await counts()).toEqual({ sale: 0, line: 0, tender: 0, mv: 0 });
  });

  test("re-posting the same sale creates no duplicate tender or movement", async () => {
    const s = await seed();
    expect((await post(s.token, sale(s))).status).toBe(200);
    expect((await post(s.token, sale(s))).status).toBe(200);
    expect(await counts()).toEqual({ sale: 1, line: 1, tender: 1, mv: 1 });
  });

  test("two lines of one product make one movement with the summed quantity", async () => {
    const s = await seed();
    const res = await post(s.token, sale(s, {
      lines: [
        { itemSeq: 1, productId: s.productId, qty: "1.250", prc: "10.00", taxTyCd: "B" },
        { itemSeq: 2, productId: s.productId, qty: "2.000", prc: "10.00", taxTyCd: "B" },
      ],
    }));
    expect(res.status).toBe(200);
    const mv = await prisma.stockMovement.findMany({ where: { type: "SALE" } });
    expect(mv).toHaveLength(1);
    expect(mv[0].qty.toFixed(3)).toBe("3.250");
    expect(await prisma.saleLine.count()).toBe(2);
  });

  test("a sale for an unknown shift is refused and nothing is written", async () => {
    const s = await seed();
    const res = await post(s.token, sale(s, { shiftId: "f".repeat(32) }));
    expect(res.status).toBe(422);
    expect(await counts()).toEqual({ sale: 0, line: 0, tender: 0, mv: 0 });
  });

  test("lastInvcNo advances, and never goes backwards on an older sale", async () => {
    const s = await seed();
    await post(s.token, sale(s, { invcNo: 7 }));
    expect((await prisma.lane.findUniqueOrThrow({ where: { id: s.lane.id } })).lastInvcNo).toBe(7);
    await post(s.token, sale(s, { saleId: "c".repeat(32), invcNo: 3 }));
    expect((await prisma.lane.findUniqueOrThrow({ where: { id: s.lane.id } })).lastInvcNo).toBe(7);
    await post(s.token, sale(s, { saleId: "d".repeat(32), invcNo: 9 }));
    expect((await prisma.lane.findUniqueOrThrow({ where: { id: s.lane.id } })).lastInvcNo).toBe(9);
  });

  test("a line naming a hard-deleted product is refused with 422", async () => {
    const s = await seed();
    const gone = await prisma.product.create({ data: { sku: "GONE", name: "Gone", unitPrice: "1.00", orgId: s.orgId } });
    await prisma.product.delete({ where: { id: gone.id } });
    const res = await post(s.token, sale(s, { lines: [{ itemSeq: 1, productId: gone.id, qty: "1.000", prc: "1.00", taxTyCd: "B" }] }));
    expect(res.status).toBe(422);
    expect(await counts()).toEqual({ sale: 0, line: 0, tender: 0, mv: 0 });
  });

  test("a line naming another org's product is refused with 422", async () => {
    const s = await seed();
    const foreign = await prisma.product.create({ data: { sku: "F", name: "F", unitPrice: "1.00", orgId: s.otherOrgId } });
    const res = await post(s.token, sale(s, { lines: [{ itemSeq: 1, productId: foreign.id, qty: "1.000", prc: "1.00", taxTyCd: "B" }] }));
    expect(res.status).toBe(422);
    expect((await counts()).sale).toBe(0);
  });

  test("a line naming a soft-deleted product is accepted", async () => {
    const s = await seed();
    await prisma.product.update({ where: { id: s.productId }, data: { deletedAt: new Date() } });
    expect((await post(s.token, sale(s))).status).toBe(200);
    expect(await counts()).toEqual({ sale: 1, line: 1, tender: 1, mv: 1 });
  });

  test("without a productId the line resolves by SKU within the org; unknown SKU is 422", async () => {
    const s = await seed();
    const p = await prisma.product.findUniqueOrThrow({ where: { id: s.productId } });
    const line = (itemCd: string) => [{ itemSeq: 1, itemCd, qty: "1.000", prc: "1.00", taxTyCd: "B" }];
    expect((await post(s.token, sale(s, { lines: line("NOPE") }))).status).toBe(422);
    // a SKU that exists only in the other org does not resolve
    await prisma.product.create({ data: { sku: "THEIRS", name: "T", unitPrice: "1.00", orgId: s.otherOrgId } });
    expect((await post(s.token, sale(s, { lines: line("THEIRS") }))).status).toBe(422);
    expect((await counts()).sale).toBe(0);
    expect((await post(s.token, sale(s, { lines: line(p.sku) }))).status).toBe(200);
    expect((await prisma.saleLine.findFirstOrThrow()).productId).toBe(s.productId);
  });

  test("money arrives as exact decimals: 1200.00 stays 1200.00", async () => {
    const s = await seed();
    await post(s.token, sale(s, { totAmt: "9999999999.99", lines: [{ itemSeq: 1, productId: s.productId, qty: "0.001", prc: "1200.00", taxTyCd: "B" }] }));
    const row = await prisma.sale.findUniqueOrThrow({ where: { id: SALE } });
    expect(row.totAmt.toFixed(2)).toBe("9999999999.99");
    const l = await prisma.saleLine.findFirstOrThrow();
    expect(l.prc.toFixed(2)).toBe("1200.00");
    expect(l.qty.toFixed(3)).toBe("0.001");
    expect((await post(s.token, sale(s, { saleId: "e".repeat(32), invcNo: 6, totAmt: 1200 }))).status).toBe(422);
  });

  test("soldAt comes from the payload, not the server clock", async () => {
    const s = await seed();
    await post(s.token, sale(s, { soldAt: "2026-01-02T03:04:05.000Z" }));
    expect((await prisma.sale.findUniqueOrThrow({ where: { id: SALE } })).soldAt.toISOString()).toBe("2026-01-02T03:04:05.000Z");
    expect((await post(s.token, sale(s, { saleId: "c".repeat(32), invcNo: 8, soldAt: "2026-01-02" }))).status).toBe(422);
  });

  test("tenant comes from the token; another lane's shift is refused", async () => {
    const s = await seed();
    const l2 = await prisma.lane.create({ data: { code: "L2", name: "L2", locationId: s.locationId, orgId: s.orgId } });
    const amy = await prisma.user.findFirstOrThrow({ where: { role: "CASHIER" } });
    await prisma.shift.create({ data: { id: "9".repeat(32), laneId: l2.id, orgId: s.orgId, openedById: amy.id, openingFloat: "0.00" } });
    expect((await post(s.token, sale(s, { shiftId: "9".repeat(32) }))).status).toBe(422);
    await post(s.token, sale(s, { orgId: s.otherOrgId, laneId: l2.id }));
    const row = await prisma.sale.findUniqueOrThrow({ where: { id: SALE } });
    expect(row.orgId).toBe(s.orgId);
    expect(row.laneId).toBe(s.lane.id);
  });

  test("a different sale reusing an invcNo is refused", async () => {
    const s = await seed();
    await post(s.token, sale(s));
    expect((await post(s.token, sale(s, { saleId: "c".repeat(32) }))).status).toBe(422);
    expect((await counts()).sale).toBe(1);
  });

  test("malformed bodies are 422", async () => {
    const s = await seed();
    const bad = [
      {}, sale(s, { saleId: "short" }), sale(s, { invcNo: 0 }), sale(s, { lines: [] }), sale(s, { tenders: undefined }),
      sale(s, { tenders: [{ pmtTyCd: "BITCOIN", amount: "1.00" }] }),
      sale(s, { lines: [{ itemSeq: 1, productId: s.productId, qty: "1e3", prc: "1.00", taxTyCd: "B" }] }),
      sale(s, { lines: [{ itemSeq: 1, productId: s.productId, qty: "1.000", prc: "1.00", taxTyCd: "Z" }] }),
    ];
    for (const b of bad) expect((await post(s.token, b)).status).toBe(422);
    expect((await counts()).sale).toBe(0);
  });

  test("a database fault is 5xx (retryable) and writes nothing", async () => {
    const s = await seed();
    const spy = vi.spyOn(prisma.product, "findMany").mockRejectedValueOnce(new Error("connection refused"));
    const res = await post(s.token, sale(s));
    spy.mockRestore();
    expect(res.status).toBeGreaterThanOrEqual(500);
    expect(await counts()).toEqual({ sale: 0, line: 0, tender: 0, mv: 0 });
  });

  test("a failure mid-transaction leaves no half-written sale", async () => {
    const s = await seed();
    await prisma.stockLevel.create({ data: { productId: s.productId, locationId: s.laneLocationId, qty: "10.000" } });
    // Inject a fault on the LAST write of the transaction, after the sale, lines, tenders,
    // movement and stock decrement have all been written.
    const real = prisma.$transaction.bind(prisma) as any;
    const spy = vi.spyOn(prisma, "$transaction").mockImplementationOnce(((fn: any, opts: any) =>
      real((tx: any) =>
        fn(new Proxy(tx, {
          get: (target, key) =>
            key === "lane" ? { updateMany: async () => { throw new Error("injected"); } } : target[key],
        })), opts)) as any);
    const res = await post(s.token, sale(s));
    spy.mockRestore();
    expect(res.status).toBe(500);
    expect(await counts()).toEqual({ sale: 0, line: 0, tender: 0, mv: 0 });
    expect((await prisma.stockLevel.findFirstOrThrow({ where: { productId: s.productId } })).qty.toFixed(3)).toBe("10.000");
    expect((await prisma.lane.findUniqueOrThrow({ where: { id: s.lane.id } })).lastInvcNo).toBe(0);
  });

  test("a missing or revoked token is 401 and writes nothing", async () => {
    const s = await seed();
    expect((await post(null, sale(s))).status).toBe(401);
    expect((await post("bogus", sale(s))).status).toBe(401);
    expect((await counts()).sale).toBe(0);
  });
});
