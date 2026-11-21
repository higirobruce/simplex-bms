import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

// Each test starts from an empty database (setup-each truncates per test), so build what it needs.
async function seedTill() {
  const s = await seedOrg();
  const mkLane = (code: string) =>
    prisma.lane.create({
      data: { code, name: `Lane ${code}`, locationId: s.locationId, orgId: s.orgId },
    });
  const laneA = await mkLane("L1");
  const laneB = await mkLane("L2");
  const shiftA = await prisma.shift.create({
    data: { laneId: laneA.id, openedById: s.userId, openingFloat: "10000", orgId: s.orgId },
  });
  const shiftB = await prisma.shift.create({
    data: { laneId: laneB.id, openedById: s.userId, openingFloat: "10000", orgId: s.orgId },
  });
  // Required Sale fields only; tests override laneId/shiftId/invcNo as needed.
  const base = {
    shiftId: shiftA.id,
    laneId: laneA.id,
    invcNo: 1,
    totAmt: "5000",
    totTaxblAmt: "4237.29",
    totTaxAmt: "762.71",
    orgId: s.orgId,
  };
  const sale = await prisma.sale.create({ data: { ...base, invcNo: 1000 } });
  return { ...s, laneA: laneA.id, laneB: laneB.id, shiftA: shiftA.id, shiftB: shiftB.id, base, saleId: sale.id };
}

test("invoice number is unique per lane, not per org", async () => {
  const { base, laneA, laneB, shiftB } = await seedTill();
  await prisma.sale.create({ data: { ...base, laneId: laneA, invcNo: 1 } });
  await expect(
    prisma.sale.create({ data: { ...base, laneId: laneA, invcNo: 1 } }),
  ).rejects.toThrow(/Unique constraint/);
  const b = await prisma.sale.create({
    data: { ...base, laneId: laneB, shiftId: shiftB, invcNo: 1 },
  });
  expect(b.invcNo).toBe(1); // two lanes legitimately both have invoice 1
});

test("a sale line holds a weighed quantity and its own tax type", async () => {
  const { saleId, productId, orgId } = await seedTill();
  const l = await prisma.saleLine.create({
    data: { saleId, productId, itemSeq: 1, qty: "0.734", prc: "1200", taxTyCd: "B", orgId },
  });
  expect(l.qty.toString()).toBe("0.734");
  expect(l.taxTyCd).toBe("B");
});

test("a sale line item sequence is unique within its sale", async () => {
  const { saleId, productId, orgId } = await seedTill();
  const data = { saleId, productId, itemSeq: 1, qty: "1", prc: "1200", taxTyCd: "B" as const, orgId };
  await prisma.saleLine.create({ data });
  await expect(prisma.saleLine.create({ data })).rejects.toThrow(/Unique constraint/);
});

test("mobile money is recorded unconfirmed and with no provider reference", async () => {
  const { saleId, orgId } = await seedTill();
  const t = await prisma.tender.create({
    data: { saleId, pmtTyCd: "MOBILE_MONEY", amount: "5000", orgId },
  });
  expect(t.confirmed).toBe(false);
  expect(t.providerRef).toBeNull();
});

test("all six RRA payment methods exist", async () => {
  const { saleId, orgId } = await seedTill();
  const all = ["CASH", "CREDIT", "CASH_CREDIT", "BANK_CHECK", "CARD", "MOBILE_MONEY"] as const;
  for (const m of all) {
    const t = await prisma.tender.create({ data: { saleId, pmtTyCd: m, amount: "1", orgId } });
    expect(t.pmtTyCd).toBe(m);
  }
});

test("a sale may carry several tenders that together exceed no constraint", async () => {
  const { saleId, orgId } = await seedTill();
  await prisma.tender.create({ data: { saleId, pmtTyCd: "CASH", amount: "3000", orgId } });
  await prisma.tender.create({ data: { saleId, pmtTyCd: "MOBILE_MONEY", amount: "2000", orgId } });
  const ts = await prisma.tender.findMany({ where: { saleId } });
  expect(ts.length).toBeGreaterThanOrEqual(2);
});

test("a sale is voided, never deleted", async () => {
  const { base } = await seedTill();
  const s = await prisma.sale.create({ data: { ...base, invcNo: 99 } });
  const v = await prisma.sale.update({ where: { id: s.id }, data: { voidedAt: new Date() } });
  expect(v.voidedAt).not.toBeNull();
});

test("a lane code is unique per org", async () => {
  const { orgId, otherOrgId, locationId } = await seedTill();
  await expect(
    prisma.lane.create({ data: { code: "L1", name: "dup", locationId, orgId } }),
  ).rejects.toThrow(/Unique constraint/);
  const other = await prisma.lane.create({ data: { code: "L1", name: "other org", locationId, orgId: otherOrgId } });
  expect(other.lastInvcNo).toBe(0);
});

test("sales expose their lines and tenders through lines and tenders", async () => {
  const { saleId, productId, orgId } = await seedTill();
  await prisma.saleLine.create({
    data: { saleId, productId, itemSeq: 1, qty: "1", prc: "1200", taxTyCd: "A", orgId },
  });
  await prisma.tender.create({ data: { saleId, pmtTyCd: "CASH", amount: "1200", orgId } });
  const [s] = await prisma.sale.findMany({ where: { id: saleId }, include: { lines: true, tenders: true } });
  expect(s.lines).toHaveLength(1);
  expect(s.tenders).toHaveLength(1);
});

test("two lanes in one org cannot share a bhfId, but another org may reuse it", async () => {
  const { orgId, locationId, otherOrgId } = await seedTill();
  await prisma.lane.create({ data: { code: "B1", name: "b1", locationId, orgId, bhfId: "00" } });
  await expect(
    prisma.lane.create({ data: { code: "B2", name: "b2", locationId, orgId, bhfId: "00" } }),
  ).rejects.toThrow(/Unique constraint/);
  const other = await prisma.lane.create({
    data: { code: "B1", name: "other org", locationId, orgId: otherOrgId, bhfId: "00" },
  });
  expect(other.bhfId).toBe("00");
});

test("any number of lanes with no bhfId are fine", async () => {
  const { orgId, locationId } = await seedTill();
  // seedTill already made two lanes with a NULL bhfId; add more
  await prisma.lane.create({ data: { code: "N1", name: "n1", locationId, orgId } });
  await prisma.lane.create({ data: { code: "N2", name: "n2", locationId, orgId, bhfId: null } });
  const n = await prisma.lane.count({ where: { orgId, bhfId: null } });
  expect(n).toBe(4);
});
