import { Prisma } from "@prisma/client";

/**
 * Quantity arithmetic for Decimal(12, 3) columns.
 *
 * Prisma.Decimal.valueOf() returns a string, so the native `+` concatenates
 * ("10" + 5 -> "105") while `-` and `*` coerce to numbers. Never add
 * quantities with `+`; route every addition through addQty.
 */
export type QtyLike = Prisma.Decimal | number | string;

export function toQty(v: QtyLike): Prisma.Decimal {
  return new Prisma.Decimal(v);
}

export function addQty(...values: QtyLike[]): Prisma.Decimal {
  return values.reduce<Prisma.Decimal>((sum, v) => sum.plus(toQty(v)), new Prisma.Decimal(0));
}

export function subQty(a: QtyLike, b: QtyLike): Prisma.Decimal {
  return toQty(a).minus(toQty(b));
}

export function isNegative(v: QtyLike): boolean {
  return toQty(v).isNegative() && !toQty(v).isZero();
}

/** Three decimals when weighed, whole units when sold each. */
export function formatQty(v: QtyLike, isWeighed: boolean): string {
  return toQty(v).toFixed(isWeighed ? 3 : 0);
}

/** qty x unit price as an exact Decimal, rounded to the money scale (2dp). */
export function lineAmount(qty: QtyLike, unitPrice: QtyLike): Prisma.Decimal {
  return toQty(qty).times(toQty(unitPrice)).toDecimalPlaces(2);
}
