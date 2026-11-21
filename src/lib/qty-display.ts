/**
 * Client-safe quantity helpers. NO Prisma import: this module is used from
 * "use client" pages, where quantities arrive as JSON strings ("10", "0.3")
 * because Prisma.Decimal serialises to a string.
 *
 * Never compare those strings directly ("10" >= "9" is false) and never
 * subtract them as floats (0.3 - 0.1 is 0.19999999999999998). Decimal(12,3)
 * values sit well inside a double, so Number() plus rounding to 3dp is exact.
 */
export type SerialisedQty = string | number;

const round3 = (n: number) => Number(n.toFixed(3));

export function isFullyDelivered(delivered: SerialisedQty, ordered: SerialisedQty): boolean {
  return Number(delivered) >= Number(ordered);
}

export function remainingQty(ordered: SerialisedQty, delivered: SerialisedQty): number {
  return round3(Number(ordered) - Number(delivered));
}

export function sumQty(values: SerialisedQty[]): number {
  return round3(values.reduce<number>((sum, v) => sum + Number(v), 0));
}

/**
 * Parse the raw text of a quantity input. Returns null for empty or
 * non-numeric text so callers can refuse it, rather than a fallback like 1
 * silently replacing what the user typed (parseInt("0.734") || 1 stored 1).
 * Fractions are kept: "0.734" -> 0.734, "2.5" -> 2.5.
 */
export function parseQtyInput(raw: string): number | null {
  const text = raw.trim();
  if (text === "" || !/^[+-]?(\d+\.?\d*|\.\d+)$/.test(text)) return null;
  const n = Number(text);
  return Number.isFinite(n) ? n : null;
}
