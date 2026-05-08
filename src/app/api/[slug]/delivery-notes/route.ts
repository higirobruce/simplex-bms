import { prisma } from "@/lib/prisma";
import {
  getTenantContext,
  requirePermission,
  handleRouteError,
  errorResponse,
} from "@/lib/api";
import { DeliveryNoteCreateSchema } from "@/lib/validations";
import { logAudit } from "@/lib/audit";
import { nextDocNo } from "@/lib/doc-sequence";
import { NextResponse } from "next/server";

export async function GET(req: Request) {
  try {
    const { orgId } = await getTenantContext();
    const url = new URL(req.url);
    const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
    const limit = Math.min(
      100,
      Math.max(1, parseInt(url.searchParams.get("limit") || "20"))
    );
    const orderId = url.searchParams.get("orderId") || undefined;
    const status = url.searchParams.get("status") || undefined;

    const where: any = { orgId };
    if (orderId) where.orderId = orderId;
    if (status) where.status = status;

    const [data, total] = await Promise.all([
      prisma.deliveryNote.findMany({
        where,
        include: {
          order: { select: { orderNo: true, customer: { select: { name: true } } } },
          location: { select: { name: true } },
          _count: { select: { lines: true } },
        },
        orderBy: { createdAt: "desc" },
        skip: (page - 1) * limit,
        take: limit,
      }),
      prisma.deliveryNote.count({ where }),
    ]);

    return NextResponse.json({
      data,
      meta: { total, page, limit, pages: Math.ceil(total / limit) },
    });
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function POST(req: Request) {
  try {
    const { orgId, userId } = await requirePermission("sales:write");
    const body = await req.json();
    const parsed = DeliveryNoteCreateSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    const { orderId, locationId, notes, lines } = parsed.data;

    const order = await prisma.salesOrder.findFirst({
      where: { id: orderId, orgId, deletedAt: null },
      include: { lines: true },
    });
    if (!order) return errorResponse("Sales order not found", 404);
    if (order.status === "DRAFT" || order.status === "CANCELLED") {
      return errorResponse(
        "Sales order must be confirmed before delivery",
        422
      );
    }

    const location = await prisma.location.findFirst({
      where: { id: locationId, orgId },
    });
    if (!location) return errorResponse("Source warehouse not found", 404);

    // Validate lines reference order lines if provided, and qty doesn't exceed remaining.
    for (const line of lines) {
      if (line.orderLineId) {
        const ol = order.lines.find((l) => l.id === line.orderLineId);
        if (!ol) {
          return errorResponse(
            `Order line ${line.orderLineId} not found on this order`,
            422
          );
        }
        if (ol.productId !== line.productId) {
          return errorResponse(
            "Delivery line product does not match order line",
            422
          );
        }
        const remaining = ol.qty - ol.qtyDelivered;
        if (line.qty > remaining) {
          return errorResponse(
            `Cannot deliver more than remaining (${remaining}) on order line`,
            422
          );
        }
      }
    }

    const note = await prisma.$transaction(async (tx) => {
      const noteNo = await nextDocNo(tx, orgId, "DN", "DeliveryNote", "DN");
      return tx.deliveryNote.create({
        data: {
          orgId,
          noteNo,
          orderId,
          locationId,
          notes: notes || null,
          createdById: userId,
          lines: {
            create: lines.map((l) => ({
              productId: l.productId,
              orderLineId: l.orderLineId || null,
              qty: l.qty,
            })),
          },
        },
        include: { lines: true, location: true, order: true },
      });
    });

    await logAudit({
      orgId,
      userId,
      action: "CREATE",
      entityType: "DeliveryNote",
      entityId: note.id,
      after: note,
    });

    return NextResponse.json(note);
  } catch (error) {
    return handleRouteError(error);
  }
}
