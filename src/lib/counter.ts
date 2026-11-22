/**
 * Pure logic for the per-shift till roll-up. No database: strings, dates and
 * exact decimals only, so it is testable without one.
 *
 * Governing rule: never produce a variance that cannot be stood behind. A
 * shift is only "complete" when what we hold provably equals what the till
 * says it sold; every other state yields no variance at all.
 */
import { Prisma } from "@prisma/client";

const D = (v: string | number) => new Prisma.Decimal(v);

export type ShiftState = "open" | "stale" | "complete" | "short_count" | "unknown" | "mismatch";

/** An open shift with no contact for this long is stale. */
export const STALE_AFTER_MS = 20 * 60 * 1000;

export function shiftState(input: {
  closedAt: Date | null;
  /** Newest authenticated contact from the lane: it means REACHABLE, not "sent a sale". */
  lastSeenAt: Date | null;
  saleCount: number | null;
  gross: string | null;
  heldCount: number;
  heldGross: string;
  now: Date;
  /** When the shift opened. With dayStart, an open shift from before today is stale. */
  openedAt?: Date;
  /** Start of the current day in the org's timezone. */
  dayStart?: Date;
}): ShiftState {
  if (!input.closedAt) {
    if (input.openedAt && input.dayStart && input.openedAt < input.dayStart) return "stale";
    // A lane that has never been heard from is not reachable.
    if (!input.lastSeenAt) return "stale";
    return input.now.getTime() - input.lastSeenAt.getTime() > STALE_AFTER_MS ? "stale" : "open";
  }
  // Both null = the till did not say; one alone is no better. Zero is a real value.
  if (input.saleCount === null || input.gross === null) return "unknown";
  if (input.saleCount > input.heldCount) return "short_count";
  if (input.saleCount < input.heldCount) return "mismatch";
  return D(input.gross).equals(D(input.heldGross)) ? "complete" : "mismatch";
}

/** expected = float + cash tendered - change given; diff = declared - expected (negative is short). */
export function variance(input: {
  openingFloat: string;
  cashTendered: string;
  changeGiven: string;
  declaredCash: string | null;
}): { expected: string; declared: string | null; diff: string | null } {
  const expected = D(input.openingFloat).plus(input.cashTendered).minus(input.changeGiven);
  if (input.declaredCash === null) return { expected: expected.toFixed(2), declared: null, diff: null };
  const declared = D(input.declaredCash);
  return { expected: expected.toFixed(2), declared: declared.toFixed(2), diff: declared.minus(expected).toFixed(2) };
}

export type SaleForSplit = { totAmt: string; tenders: { pmtTyCd: string; amount: string }[] };

/**
 * Tenders record what the customer HANDED OVER, so per method they exceed
 * takings by the change given. The split is therefore gross `methods` minus
 * `changeGiven` (one deduction), and `reconciles` says whether
 * sum(methods) - changeGiven equals takings (sum of totAmt).
 * Change per sale = (sum of its tenders - totAmt), floored at 0 and capped by
 * the cash taken on that sale (change is paid out of cash only).
 */
export function splitTakings(sales: SaleForSplit[]) {
  const byMethod = new Map<string, Prisma.Decimal>();
  let takings = D(0);
  let change = D(0);
  for (const s of sales) {
    const tot = D(s.totAmt);
    takings = takings.plus(tot);
    let tendered = D(0);
    let cash = D(0);
    for (const t of s.tenders) {
      const a = D(t.amount);
      tendered = tendered.plus(a);
      if (t.pmtTyCd === "CASH") cash = cash.plus(a);
      byMethod.set(t.pmtTyCd, (byMethod.get(t.pmtTyCd) ?? D(0)).plus(a));
    }
    const over = tendered.minus(tot);
    if (over.greaterThan(0)) change = change.plus(Prisma.Decimal.min(over, cash));
  }
  let methodSum = D(0);
  const methods = Array.from(byMethod.entries())
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([method, amt]) => {
      methodSum = methodSum.plus(amt);
      return { method, tendered: amt.toFixed(2) };
    });
  const cashTendered = byMethod.get("CASH") ?? D(0);
  return {
    methods,
    changeGiven: change.toFixed(2),
    takings: takings.toFixed(2),
    cashTendered: cashTendered.toFixed(2),
    reconciles: methodSum.minus(change).equals(takings),
    // The shift's gross (what is held), for the state check.
    heldGross: takings.toFixed(2),
  };
}

/** The calendar date (YYYY-MM-DD) `instant` falls on in `tz`. */
export function localDate(instant: Date, tz: string): string {
  return new Intl.DateTimeFormat("en-CA", { timeZone: tz, year: "numeric", month: "2-digit", day: "2-digit" }).format(instant);
}

/** An instant that falls on calendar day `day` (YYYY-MM-DD) in `tz`, or null if `day` is not a real date. */
export function instantOnDay(day: string, tz: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const probe = new Date(Date.UTC(y, mo - 1, d));
  if (probe.getUTCFullYear() !== y || probe.getUTCMonth() !== mo - 1 || probe.getUTCDate() !== d) return null;
  // Zones span UTC-12..+14, so one of these hours lands on the wanted local date.
  for (const h of [12, 0, 23, 6, 18]) {
    const t = new Date(Date.UTC(y, mo - 1, d, h));
    if (localDate(t, tz) === day) return t;
  }
  return null;
}
