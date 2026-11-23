import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

test("every RRA payment code 01-07 is representable as a TenderMethod", async () => {
  // VSDC spec v1.0.4 §4.10: seven codes. Phase 1 shipped six.
  const { orgId, productId, userId, locationId } = await seedOrg();
  const lane = await prisma.lane.create({ data: { code: "L1", name: "Lane 1", locationId, orgId, bhfId: "00" } });
  const shift = await prisma.shift.create({ data: { laneId: lane.id, openedById: userId, openingFloat: "0", orgId } });
  const sale = await prisma.sale.create({
    data: { shiftId: shift.id, laneId: lane.id, invcNo: 1, totAmt: "0", totTaxblAmt: "0", totTaxAmt: "0", orgId },
  });
  const all = ["CASH", "CREDIT", "CASH_CREDIT", "BANK_CHECK", "CARD", "MOBILE_MONEY", "OTHER"] as const;
  for (const m of all) {
    const t = await prisma.tender.create({ data: { saleId: sale.id, seq: all.indexOf(m), pmtTyCd: m, amount: "0", orgId } });
    expect(t.pmtTyCd).toBe(m);
  }
});
