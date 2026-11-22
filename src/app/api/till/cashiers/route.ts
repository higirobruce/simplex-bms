import { prisma } from "@/lib/prisma";
import { handleRouteError } from "@/lib/api";
import { getLaneContext } from "@/lib/lane-auth";
import { NextResponse } from "next/server";

// Till-only. Authenticated by a lane bearer token (getLaneContext), never by a browser
// session. This response DELIBERATELY carries each cashier's argon2id pinHash: the till
// verifies PINs offline, which is impossible without it. That is why this route must stay
// token-only and why no /api/[slug]/... route may ever return a pinHash.
// `active` is derived (pinHash !== null); a deactivated cashier has no hash to send.
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  try {
    const { orgId } = await getLaneContext(req);
    const rows = await prisma.user.findMany({
      where: { orgId, role: "CASHIER" },
      select: { id: true, name: true, pinHash: true },
      orderBy: { name: "asc" },
    });
    const body = rows.map((r) => ({ id: r.id, name: r.name, pinHash: r.pinHash, active: r.pinHash !== null }));
    return NextResponse.json(body, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return handleRouteError(error);
  }
}
