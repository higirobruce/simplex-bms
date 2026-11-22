import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

async function makeShift(orgId: string, extra: { saleCount?: number; gross?: string } = {}) {
  const { id: locationId } = await prisma.location.findFirstOrThrow({ where: { orgId } });
  const lane = await prisma.lane.create({ data: { code: "L1", name: "L1", locationId, orgId } });
  return prisma.shift.create({
    data: { laneId: lane.id, openedById: "u1", openingFloat: "0", orgId, ...extra },
  });
}

test("a shift can be created with neither field, meaning unknown", async () => {
  const { orgId } = await seedOrg();
  const shift = await makeShift(orgId);
  expect(shift.saleCount).toBeNull();
  expect(shift.gross).toBeNull();
});

test("both fields round-trip as exact decimals", async () => {
  const { orgId } = await seedOrg();
  const { id } = await makeShift(orgId, { saleCount: 0, gross: "1234567.89" });
  const shift = await prisma.shift.findUniqueOrThrow({ where: { id } });
  expect(shift.saleCount).toBe(0);
  expect(shift.gross?.toString()).toBe("1234567.89");
});
