/**
 * Presentation helpers for the counter page. Pure: strings in, strings out.
 * Money arrives from the API as decimal strings and is only ever re-grouped
 * as text here, never parsed to a float.
 */

import type { ShiftState } from "@/lib/counter";
export type { ShiftState };

export type CounterShift = {
  id: string;
  lane: { id: string; code: string; name: string };
  cashier: { id: string; name: string | null };
  openedAt: string;
  closedAt: string | null;
  lastReachedAt: string | null;
  state: ShiftState;
  openingFloat: string;
  saleCount: number;
  tillSaleCount: number | null;
  takings: string;
  tillGross: string | null;
  methods: { method: string; tendered: string }[];
  changeGiven: string;
  variance: { expected: string; declared: string | null; diff: string | null } | null;
};

export type CounterResponse = {
  day: string;
  /** The org's IANA zone: days are bucketed in it, so times must be shown in it. */
  timezone: string;
  shifts: CounterShift[];
  total: string;
  totalQualified: boolean;
};

/** "1234567.50" -> "1,234,567.50"; whole amounts drop the ".00". Sign is kept. */
export function groupMoney(value: string): string {
  const m = /^(-?)(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!m) return value;
  const [, sign, int, frac] = m;
  const grouped = int.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const hasFrac = frac !== undefined && /[1-9]/.test(frac);
  return `${sign}${grouped}${hasFrac ? "." + frac : ""}`;
}

/** An absolute amount as text: "-2500.00" -> "2,500". Never shows a minus. */
export function absMoney(value: string): string {
  return groupMoney(value.startsWith("-") ? value.slice(1) : value);
}

const isZero = (v: string) => /^-?0+(\.0+)?$/.test(v.trim());

export type VarianceKind = "short" | "over" | "balanced";

/** Short, over or balanced from a signed diff (declared - expected). */
export function varianceKind(diff: string): VarianceKind {
  if (isZero(diff)) return "balanced";
  return diff.trim().startsWith("-") ? "short" : "over";
}

const METHOD_LABELS: Record<string, string> = {
  CASH: "Cash",
  CREDIT: "Credit",
  CASH_CREDIT: "Cash credit",
  BANK_CHECK: "Bank cheque",
  CARD: "Card",
  MOBILE_MONEY: "Mobile money",
  OTHER: "Other",
};
export const methodLabel = (m: string) => METHOD_LABELS[m] ?? m;

/** Local HH:MM (24h). `tz` is for tests; the page uses the viewer's own zone. */
export function clock(iso: string, tz?: string): string {
  return new Date(iso).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", hour12: false, timeZone: tz });
}

function dayKey(iso: string, tz?: string): string {
  return new Intl.DateTimeFormat("en-CA", { timeZone: tz, year: "numeric", month: "2-digit", day: "2-digit" }).format(new Date(iso));
}

/** "09:41", or "09:41 yesterday" / "09:41 on 2026-11-20" when not on `now`'s day. */
export function clockWithDay(iso: string, now: Date, tz?: string): string {
  const t = clock(iso, tz);
  const d = dayKey(iso, tz);
  const today = dayKey(now.toISOString(), tz);
  if (d === today) return t;
  const yesterday = dayKey(new Date(now.getTime() - 24 * 3600 * 1000).toISOString(), tz);
  return d === yesterday ? `${t} yesterday` : `${t} on ${d}`;
}

/** The YYYY-MM-DD one day before/after `day`. */
export function shiftDay(day: string, delta: number): string {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d + delta)).toISOString().slice(0, 10);
}

/** Shifts that have a verdict to read vs shifts still trading (or silently stuck). */
export function splitShifts(shifts: CounterShift[]) {
  return {
    closed: shifts.filter((s) => s.state !== "open" && s.state !== "stale"),
    live: shifts.filter((s) => s.state === "open" || s.state === "stale"),
  };
}

export type ShiftSaleLine = {
  itemSeq: number;
  name: string;
  qty: string;
  prc: string;
  taxTyCd: string;
  /** Stored by the till; null on rows that predate the columns. Never derived. */
  splyAmt: string | null;
  taxblAmt: string | null;
};

export type ShiftSale = {
  id: string;
  invcNo: number;
  rcptNo: number | null;
  soldAt: string;
  voidedAt: string | null;
  totAmt: string;
  totTaxblAmt: string;
  totTaxAmt: string;
  buyerTin: string | null;
  lines: ShiftSaleLine[];
  tenders: { seq: number; method: string; amount: string }[];
};

export type ShiftSalesResponse = {
  shiftId: string;
  record: { kind: "back-office"; fiscalCopy: false; note: string };
  sales: ShiftSale[];
  /** True number of sales on the shift; `sales` is capped. */
  salesTotal: number;
  truncated: boolean;
};

/** A trailing-zero-free quantity: "3.000" -> "3", "0.500" -> "0.5". Text only. */
export function plainQty(value: string): string {
  return /^-?\d+\.\d+$/.test(value) ? value.replace(/\.?0+$/, "") : value;
}

/** Hard cap on sales returned for one shift, so a busy shift cannot return an unbounded payload. */
export const SALES_CAP = 200;
