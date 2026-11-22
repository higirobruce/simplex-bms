import { prisma } from "@/lib/prisma";
import { handleRouteError } from "@/lib/api";
import { getLaneContext } from "@/lib/lane-auth";
import { NextResponse } from "next/server";

// Shift ingest from the till's outbox. The till marks a row sent only on 2xx, so status
// codes are load-bearing:
//   401      bad/revoked token (till stops and surfaces it; nothing is lost)
//   422      a refusal retrying can NEVER fix (malformed body, unknown / inactive / foreign
//            cashier, shift id owned by another lane/org). The till parks the row.
//   5xx      transient fault (database down...). Anything not explicitly classified below
//            falls through to handleRouteError -> 500, and the till retries.
// laneId and orgId come ONLY from the token. Any such field in the body is ignored.

const refuse = (message: string) => NextResponse.json({ error: message }, { status: 422 });

const HEX32 = /^[0-9a-f]{32}$/;
const MONEY = /^[0-9]{1,10}(\.[0-9]{1,2})?$/; // Decimal(12,2), exact string, never a float

// ISO-8601 instant with an explicit Z or numeric offset. new Date() alone accepts "2026" or "1"
// and reads zone-less strings in server-local time, which would silently mis-stamp a shift.
const ISO_INSTANT = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})$/;
function date(v: unknown): Date | null {
  if (typeof v !== "string" || !ISO_INSTANT.test(v)) return null;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? null : d;
}
const money = (v: unknown): string | null => (typeof v === "string" && MONEY.test(v) ? v : null);

export async function POST(req: Request) {
  try {
    const { laneId, orgId } = await getLaneContext(req);

    let body: any;
    try {
      body = await req.json();
    } catch {
      return refuse("Body must be JSON");
    }
    if (!body || typeof body !== "object" || Array.isArray(body)) return refuse("Body must be an object");

    if (typeof body.id !== "string" || !HEX32.test(body.id)) return refuse("id must be 32 lowercase hex characters");
    if (typeof body.openedById !== "string" || !body.openedById) return refuse("openedById is required");
    const openedAt = date(body.openedAt);
    if (!openedAt) return refuse("openedAt must be an ISO date");
    const openingFloat = money(body.openingFloat);
    if (!openingFloat) return refuse("openingFloat must be a decimal string with at most 2 places");

    const hasClosedAt = body.closedAt !== undefined && body.closedAt !== null;
    const hasDeclared = body.declaredCash !== undefined && body.declaredCash !== null;
    if (hasClosedAt !== hasDeclared) return refuse("closedAt and declaredCash must be sent together");
    let closedAt: Date | null = null;
    let declaredCash: string | null = null;
    if (hasClosedAt) {
      closedAt = date(body.closedAt);
      declaredCash = money(body.declaredCash);
      if (!closedAt) return refuse("closedAt must be an ISO date");
      if (!declaredCash) return refuse("declaredCash must be a decimal string with at most 2 places");
    }

    const existing = await prisma.shift.findUnique({
      where: { id: body.id },
      select: { laneId: true, orgId: true, openedById: true },
    });
    if (existing && (existing.laneId !== laneId || existing.orgId !== orgId)) {
      return refuse("Shift id belongs to another lane");
    }
    if (existing && existing.openedById !== body.openedById) return refuse("openedById does not match the shift");

    // The cashier must be a CASHIER of this org. A cashier deactivated after opening the
    // shift may still have it closed: the check applies when the shift is first created.
    if (!existing) {
      const cashier = await prisma.user.findFirst({
        where: { id: body.openedById, orgId, role: "CASHIER" },
        select: { pinHash: true },
      });
      if (!cashier) return refuse("Unknown cashier");
      if (cashier.pinHash === null) return refuse("Cashier is inactive");
    }

    const shift = await prisma.shift.upsert({
      where: { id: body.id },
      create: {
        id: body.id,
        laneId,
        orgId,
        openedById: body.openedById,
        openedAt,
        openingFloat,
        closedAt,
        declaredCash,
      },
      // A replayed open must not reopen a closed shift; a close fills in the two fields.
      update: closedAt ? { closedAt, declaredCash } : {},
      select: { id: true },
    });
    return NextResponse.json({ id: shift.id });
  } catch (error) {
    return handleRouteError(error);
  }
}
