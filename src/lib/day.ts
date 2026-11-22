/**
 * Calendar-day bounds in an org's own timezone (Org.timezone), never the
 * server's. The zone's real offset at each instant is read from
 * Intl.DateTimeFormat, so DST and non-hour offsets are handled; do not
 * replace with a fixed offset (Rwanda's +2 would hide the bug for other orgs).
 */

const FALLBACK_TZ = "UTC";
const warned = new Set<string>();

/** Returns tz if the runtime recognises it; otherwise warns once and returns UTC. */
function safeZone(tz: string): string {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: tz });
    return tz;
  } catch {
    if (!warned.has(tz)) {
      warned.add(tz);
      console.warn(`[day] unknown timezone ${JSON.stringify(tz)}; falling back to ${FALLBACK_TZ}`);
    }
    return FALLBACK_TZ;
  }
}

const formatters = new Map<string, Intl.DateTimeFormat>();
function formatter(tz: string): Intl.DateTimeFormat {
  let f = formatters.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      second: "numeric",
    });
    formatters.set(tz, f);
  }
  return f;
}

type Wall = { y: number; m: number; d: number; ms: number };

/** Wall-clock fields of `instant` in tz; `ms` is those fields read as if UTC. */
function wall(instant: number, tz: string): Wall {
  const p: Record<string, number> = {};
  for (const part of formatter(tz).formatToParts(new Date(instant))) {
    if (part.type !== "literal") p[part.type] = Number(part.value);
  }
  return {
    y: p.year,
    m: p.month,
    d: p.day,
    ms: Date.UTC(p.year, p.month - 1, p.day, p.hour % 24, p.minute, p.second),
  };
}

/** Offset (ms, east positive) of tz at an instant. */
function offsetAt(instant: number, tz: string): number {
  const floored = Math.floor(instant / 1000) * 1000;
  return wall(instant, tz).ms - floored;
}

/** The UTC instant at which the wall clock in tz reads y-m-d 00:00. */
function zonedMidnight(y: number, m: number, d: number, tz: string): number {
  const asUtc = Date.UTC(y, m - 1, d);
  // Two passes settle the offset when the guess lands across a DST change.
  let t = asUtc - offsetAt(asUtc, tz);
  t = asUtc - offsetAt(t, tz);
  return t;
}

/** UTC instants bounding the calendar day of `day` in `tz`; start inclusive, end exclusive. */
export function dayBounds(day: Date, tz: string): { start: Date; end: Date } {
  const zone = safeZone(tz);
  if (Number.isNaN(day.getTime())) return { start: new Date(NaN), end: new Date(NaN) };
  const { y, m, d } = wall(day.getTime(), zone);
  const next = new Date(Date.UTC(y, m - 1, d + 1));
  return {
    start: new Date(zonedMidnight(y, m, d, zone)),
    end: new Date(
      zonedMidnight(next.getUTCFullYear(), next.getUTCMonth() + 1, next.getUTCDate(), zone),
    ),
  };
}

/** The instant the month containing `day` began in `tz`. */
export function startOfMonthIn(day: Date, tz: string): Date {
  const zone = safeZone(tz);
  if (Number.isNaN(day.getTime())) return new Date(NaN);
  const { y, m } = wall(day.getTime(), zone);
  return new Date(zonedMidnight(y, m, 1, zone));
}
