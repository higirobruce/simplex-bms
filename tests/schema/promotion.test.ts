import { expect, test } from "vitest";
import { prisma, seedOrg } from "../helpers/db";

test("a promotion cannot end when it starts (CHECK endsAt > startsAt)", async () => {
  // Equal bounds isolate the CHECK: tsrange(x, x) is a legal empty range, so the
  // EXCLUDE constraint would accept this row without promotion_dates_ordered.
  const { orgId, productId } = await seedOrg();
  const at = new Date("2026-12-01");
  await expect(
    prisma.promotion.create({
      data: { productId, price: "900", startsAt: at, endsAt: at, orgId },
    }),
  ).rejects.toThrow(/promotion_dates_ordered/);
});

test("a promotion cannot end before it starts", async () => {
  // Rejected by tsrange inside the EXCLUDE index expression as well as the CHECK,
  // so this does not prove the CHECK on its own; the equal-bounds test does.
  const { orgId, productId } = await seedOrg();
  await expect(
    prisma.promotion.create({
      data: {
        productId,
        price: "900",
        startsAt: new Date("2026-12-10"),
        endsAt: new Date("2026-12-01"),
        orgId,
      },
    }),
  ).rejects.toThrow();
});

test("overlapping promotions on one product are rejected", async () => {
  const { orgId, productId } = await seedOrg();
  await prisma.promotion.create({
    data: {
      productId,
      price: "900",
      startsAt: new Date("2026-12-01"),
      endsAt: new Date("2026-12-10"),
      orgId,
    },
  });
  await expect(
    prisma.promotion.create({
      data: {
        productId,
        price: "850",
        startsAt: new Date("2026-12-05"),
        endsAt: new Date("2026-12-15"),
        orgId,
      },
    }),
  ).rejects.toThrow(/promotion_no_overlap/);
});

test("a promotion may start exactly when the previous one ends", async () => {
  const { orgId, productId } = await seedOrg();
  await prisma.promotion.create({
    data: {
      productId,
      price: "900",
      startsAt: new Date("2026-12-01"),
      endsAt: new Date("2026-12-10"),
      orgId,
    },
  });
  const next = await prisma.promotion.create({
    data: {
      productId,
      price: "850",
      startsAt: new Date("2026-12-10"),
      endsAt: new Date("2026-12-20"),
      orgId,
    },
  });
  expect(next.price.toString()).toBe("850");
});

test("a price change records both sides and who did it", async () => {
  const { orgId, productId, userId } = await seedOrg();
  const c = await prisma.priceChange.create({
    data: { productId, oldPrice: "1200", newPrice: "1150", changedById: userId, orgId },
  });
  expect(c.oldPrice.toString()).toBe("1200");
  expect(c.newPrice.toString()).toBe("1150");
  expect(c.changedById).toBe(userId);
});
