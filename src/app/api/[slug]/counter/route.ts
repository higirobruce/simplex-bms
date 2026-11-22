import { prisma } from "@/lib/prisma";
import { getTenantContext, handleRouteError } from "@/lib/api";
import { dayBounds } from "@/lib/day";
import { instantOnDay, localDate, shiftState, splitTakings, variance } from "@/lib/counter";
import { Prisma } from "@prisma/client";
import { NextResponse } from "next/server";

// Per-shift till roll-up for one calendar day (in the org's timezone), grouped
// by the day the shift OPENED. The tenant comes from the session, never the URL.
// A variance is returned only for a shift whose data is provably complete.
export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const org = await prisma.org.findUniqueOrThrow({ where: { id: orgId }, select: { timezone: true } });
    const tz = org.timezone;
    const now = new Date();

    const dayParam = new URL(req.url).searchParams.get("day");
    const day = dayParam ?? localDate(now, tz);
    const anchor = instantOnDay(day, tz);
    if (!anchor) return NextResponse.json({ error: "day must be a real date, YYYY-MM-DD" }, { status: 400 });
    const { start, end } = dayBounds(anchor, tz);
    const todayStart = dayBounds(now, tz).start;

    const shifts = await prisma.shift.findMany({
      where: { orgId, openedAt: { gte: start, lt: end } },
      orderBy: [{ openedAt: "asc" }, { id: "asc" }],
      include: {
        lane: {
          select: {
            id: true, code: true, name: true,
            tokens: { where: { revokedAt: null }, select: { lastUsedAt: true } },
          },
        },
        sales: {
          where: { voidedAt: null },
          select: { totAmt: true, tenders: { select: { pmtTyCd: true, amount: true } } },
        },
      },
    });

    // Shift.openedById has no relation to User: resolve names in a second query,
    // deactivated cashiers included (their row stays, only pinHash is cleared).
    const ids = Array.from(new Set(shifts.map((s) => s.openedById)));
    const users = ids.length
      ? await prisma.user.findMany({ where: { id: { in: ids }, orgId }, select: { id: true, name: true } })
      : [];
    const nameOf = new Map(users.map((u) => [u.id, u.name]));

    let total = new Prisma.Decimal(0);
    let totalQualified = true;
    const rows = shifts.map((s) => {
      const split = splitTakings(
        s.sales.map((x) => ({
          totAmt: x.totAmt.toFixed(2),
          tenders: x.tenders.map((t) => ({ pmtTyCd: t.pmtTyCd, amount: t.amount.toFixed(2) })),
        })),
      );
      const seen = s.lane.tokens.map((t) => t.lastUsedAt?.getTime()).filter((t): t is number => t != null);
      const lastReachedAt = seen.length ? new Date(Math.max(...seen)) : null;
      let state = shiftState({
        closedAt: s.closedAt,
        lastSeenAt: lastReachedAt,
        saleCount: s.saleCount,
        gross: s.gross ? s.gross.toFixed(2) : null,
        heldCount: s.sales.length,
        heldGross: split.heldGross,
        now,
        openedAt: s.openedAt,
        dayStart: todayStart,
      });
      // A split that does not add up is data we cannot stand behind either.
      if (state === "complete" && !split.reconciles) state = "mismatch";

      total = total.plus(split.takings);
      if (state !== "complete") totalQualified = false;

      const v =
        state === "complete" && s.declaredCash !== null
          ? variance({
              openingFloat: s.openingFloat.toFixed(2),
              cashTendered: split.cashTendered,
              changeGiven: split.changeGiven,
              declaredCash: s.declaredCash.toFixed(2),
            })
          : null;

      return {
        id: s.id,
        lane: { id: s.lane.id, code: s.lane.code, name: s.lane.name },
        cashier: { id: s.openedById, name: nameOf.get(s.openedById) ?? null },
        openedAt: s.openedAt.toISOString(),
        closedAt: s.closedAt?.toISOString() ?? null,
        // Reachable, NOT "sent a sale". Null when the lane has no live token or never called.
        lastReachedAt: lastReachedAt?.toISOString() ?? null,
        state,
        openingFloat: s.openingFloat.toFixed(2),
        saleCount: s.sales.length,
        tillSaleCount: s.saleCount,
        takings: split.takings,
        tillGross: s.gross ? s.gross.toFixed(2) : null,
        // Gross tendered per method; takings = sum(methods) - changeGiven.
        methods: split.methods,
        changeGiven: split.changeGiven,
        variance: v,
      };
    });

    return NextResponse.json({ day, shifts: rows, total: total.toFixed(2), totalQualified });
  } catch (error) {
    return handleRouteError(error);
  }
}
