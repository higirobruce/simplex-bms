import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { InvoiceStatusTransitionSchema } from "@/lib/validations";
import { NextResponse } from "next/server";
import { InvoiceStatus } from "@prisma/client";

const TRANSITIONS: Record<string, Record<string, InvoiceStatus>> = {
  send:          { DRAFT: "SENT" },
  mark_overdue:  { SENT: "OVERDUE", PARTIAL: "OVERDUE" },
  void:          { DRAFT: "VOID", SENT: "VOID", PARTIAL: "VOID", OVERDUE: "VOID" },
  reopen:        { VOID: "DRAFT" },
};

export async function POST(req: Request, { params }: { params: { id: string } }) {
  try {
    const body = await req.json();
    const parsed = InvoiceStatusTransitionSchema.safeParse(body);
    if (!parsed.success) {
      return errorResponse(parsed.error.issues[0].message, 422);
    }

    const { action } = parsed.data;

    // Void requires special permission
    const permission = action === "void" ? "invoices:void" : "invoices:write";
    const { orgId } = await requirePermission(permission);

    const invoice = await prisma.invoice.findFirst({
      where: { id: params.id, orgId },
    });
    if (!invoice) return errorResponse("Invoice not found", 404);

    const allowedTransitions = TRANSITIONS[action];
    const newStatus = allowedTransitions?.[invoice.status];
    if (!newStatus) {
      return errorResponse(
        `Cannot ${action} an invoice with status "${invoice.status}"`,
        409
      );
    }

    const updateData: any = { status: newStatus };
    if (action === "send") {
      updateData.sentAt = new Date();
    }

    const updated = await prisma.invoice.update({
      where: { id: params.id },
      data: updateData,
    });

    return NextResponse.json(updated);
  } catch (error) {
    return handleRouteError(error);
  }
}
