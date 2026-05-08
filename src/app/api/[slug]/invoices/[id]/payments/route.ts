import { prisma } from "@/lib/prisma";
import { requirePermission, handleRouteError, errorResponse } from "@/lib/api";
import { PaymentCreateSchema } from "@/lib/validations";
import { NextResponse } from "next/server";

export async function POST(req: Request, { params }: { params: { id: string } }) {
  try {
    const { orgId } = await requirePermission("payments:write");
    const body = await req.json();
    const parsed = PaymentCreateSchema.safeParse(body);
    if (!parsed.success) {
      return errorResponse(parsed.error.issues[0].message, 422);
    }

    const { amount, method, notes } = parsed.data;

    const result = await prisma.$transaction(async (tx) => {
      // Fetch invoice with lock (inside transaction)
      const invoice = await tx.invoice.findFirst({
        where: { id: params.id, orgId },
      });
      if (!invoice) throw new Error("Invoice not found");

      if (invoice.status === "VOID") {
        throw new Error("Cannot record payment on a voided invoice");
      }
      if (invoice.status === "PAID") {
        throw new Error("Invoice is already fully paid");
      }

      const balance = Number(invoice.total) - Number(invoice.amountPaid);
      if (amount > balance + 0.01) {
        throw new Error(`Payment amount (${amount}) exceeds outstanding balance (${balance.toFixed(2)})`);
      }

      const payment = await tx.payment.create({
        data: {
          invoiceId: params.id,
          amount,
          method,
          notes: notes || null,
        },
      });

      const newAmountPaid = Number(invoice.amountPaid) + amount;
      const newStatus = newAmountPaid >= Number(invoice.total) ? "PAID" : "PARTIAL";

      await tx.invoice.update({
        where: { id: params.id },
        data: { amountPaid: newAmountPaid, status: newStatus },
      });

      // Update customer credit balance
      await tx.customer.update({
        where: { id: invoice.customerId },
        data: { creditBalance: { decrement: amount } },
      });

      return payment;
    });

    return NextResponse.json(result);
  } catch (error) {
    if (error instanceof Error && error.message.includes("not found")) {
      return errorResponse(error.message, 404);
    }
    return handleRouteError(error);
  }
}
