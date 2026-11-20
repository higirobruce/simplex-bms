import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { StockAdjustSchema } from "@/lib/validations";
import { addQty, formatQty, isNegative, toQty } from "@/lib/qty";
import { NextResponse } from "next/server";

export async function POST(req: Request) {
  try {
    const { orgId } = await requirePermission("stock:adjust");
    const body = await req.json();
    const parsed = StockAdjustSchema.safeParse(body);
    if (!parsed.success) {
      return errorResponse(parsed.error.issues[0].message, 422);
    }

    const { productId, locationId, qty, type, reason } = parsed.data;

    // Verify product belongs to org
    const product = await prisma.product.findFirst({
      where: { id: productId, orgId },
    });
    if (!product) return errorResponse("Product not found", 404);

    // Verify location belongs to org
    const location = await prisma.location.findFirst({
      where: { id: locationId, orgId },
    });
    if (!location) return errorResponse("Location not found", 404);

    const result = await prisma.$transaction(async (tx) => {
      const current = await tx.stockLevel.findUnique({
        where: { productId_locationId: { productId, locationId } },
      });

      const currentQty = toQty(current?.qty ?? 0);
      const newQty = addQty(currentQty, qty);

      if (isNegative(newQty)) {
        throw new Error(
          `Insufficient stock. Available: ${formatQty(currentQty, true)}, requested: ${formatQty(toQty(qty).abs(), true)}`
        );
      }

      await tx.stockLevel.upsert({
        where: { productId_locationId: { productId, locationId } },
        update: { qty: newQty },
        create: { productId, locationId, qty: newQty },
      });

      const movement = await tx.stockMovement.create({
        data: {
          productId,
          toLocationId: qty > 0 ? locationId : null,
          fromLocationId: qty < 0 ? locationId : null,
          qty: Math.abs(qty),
          type,
          reason: reason || null,
          orgId,
        },
      });

      return movement;
    });

    return NextResponse.json(result);
  } catch (error) {
    if (error instanceof Error && error.message.includes("Insufficient stock")) {
      return errorResponse(error.message, 400);
    }
    return handleRouteError(error);
  }
}
