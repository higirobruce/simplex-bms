import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { logAudit } from "@/lib/audit";
import { hashPin, validatePin, syntheticEmail, UNUSABLE_PASSWORD, PIN_MIN_LENGTH } from "@/lib/cashier";
import { NextResponse } from "next/server";

// Cashier accounts. ADMIN only (org:write). The org always comes from the session,
// never from the body or URL slug. The PIN is argon2id-hashed into pinHash; the
// password column holds an unusable value so a cashier can never log in on the web.
// PIN and pinHash are never returned, logged or audited. A cashier is "active"
// iff pinHash is set; deactivating clears it (no PIN can then be verified).

const SELECT = { id: true, name: true, createdAt: true } as const;
const pinError = `PIN must be ${PIN_MIN_LENGTH} to 12 digits, digits only`;

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("org:write");
    const body = await req.json().catch(() => null);
    const name = typeof body?.name === "string" ? body.name.trim().slice(0, 100) : "";
    if (!name) return errorResponse("name is required", 400);
    if (!validatePin(body?.pin)) return errorResponse(pinError, 400);

    const org = await prisma.org.findUniqueOrThrow({ where: { id: orgId }, select: { slug: true } });
    const user = await prisma.user.create({
      data: {
        name,
        email: syntheticEmail(name, org.slug),
        password: UNUSABLE_PASSWORD,
        pinHash: await hashPin(body.pin),
        role: "CASHIER",
        orgId,
      },
      select: SELECT,
    });
    await logAudit({ orgId, userId, action: "CREATE", entityType: "Cashier", entityId: user.id, after: { name } });
    return NextResponse.json(user, { status: 201 });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function GET() {
  try {
    const { orgId } = await requirePermission("org:write");
    const rows = await prisma.user.findMany({
      where: { orgId, role: "CASHIER" },
      select: { ...SELECT, pinHash: true },
      orderBy: { createdAt: "desc" },
    });
    const data = rows.map(({ pinHash, ...r }) => ({ ...r, active: pinHash !== null }));
    return NextResponse.json({ data });
  } catch (error) {
    return handleRouteError(error);
  }
}

// Body: { id, pin? , deactivate? }. `pin` sets a new PIN (and reactivates);
// `deactivate: true` clears the PIN.
export async function PATCH(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("org:write");
    const body = await req.json().catch(() => null);
    if (typeof body?.id !== "string") return errorResponse("id is required", 400);
    const deactivate = body.deactivate === true;
    if (deactivate === (body.pin !== undefined)) return errorResponse("provide either pin or deactivate", 400);
    if (!deactivate && !validatePin(body.pin)) return errorResponse(pinError, 400);

    const existing = await prisma.user.findFirst({
      where: { id: body.id, orgId, role: "CASHIER" },
      select: { id: true },
    });
    if (!existing) return errorResponse("Cashier not found", 404);

    const pinHash = deactivate ? null : await hashPin(body.pin);
    await prisma.user.update({ where: { id: existing.id }, data: { pinHash } });
    await logAudit({
      orgId,
      userId,
      action: "UPDATE",
      entityType: "Cashier",
      entityId: existing.id,
      after: { active: !deactivate, pinChanged: !deactivate },
    });
    return NextResponse.json({ id: existing.id, active: !deactivate });
  } catch (error) {
    return handleRouteError(error);
  }
}
