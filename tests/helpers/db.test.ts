import { expect, test } from "vitest";
import { prisma, resetDb, seedOrg } from "./db";

test("the test client is not pointed at the dev database", () => {
  expect(process.env.DATABASE_URL).toContain("bms_test");
  expect(process.env.DATABASE_URL).not.toMatch(/\/bms(\?|$)/);
});

test("resetDb leaves no rows behind", async () => {
  await seedOrg();
  expect(await prisma.org.count()).toBeGreaterThan(0);
  await resetDb();
  expect(await prisma.org.count()).toBe(0);
  expect(await prisma.product.count()).toBe(0);
});

test("seedOrg produces two distinct orgs", async () => {
  const f = await seedOrg();
  expect(f.orgId).not.toBe(f.otherOrgId);
  const p = await prisma.product.findUnique({ where: { id: f.productId } });
  expect(p!.orgId).toBe(f.orgId);
  const l = await prisma.location.findUnique({ where: { id: f.locationId } });
  expect(l!.orgId).toBe(f.orgId);
  const u = await prisma.user.findUnique({ where: { id: f.userId } });
  expect(u!.orgId).toBe(f.orgId);
});

// Order-dependent: only proves the beforeEach reset when it runs after the seeding tests above
// in the same file. Run alone or with -t it passes vacuously. Keep it: it is the only check
// on the setup-each.ts wiring.
test("each test starts from an empty database", async () => {
  expect(await prisma.org.count()).toBe(0);
});
