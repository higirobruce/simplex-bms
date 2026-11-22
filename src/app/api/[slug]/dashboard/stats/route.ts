import { prisma } from "@/lib/prisma";
import { getTenantContext, handleRouteError } from "@/lib/api";
import { startOfMonthIn } from "@/lib/day";
import { Prisma } from "@prisma/client";
import { NextResponse } from "next/server";

export async function GET() {
  try {
    const { orgId } = await getTenantContext();

    const now = new Date();
    const org = await prisma.org.findUniqueOrThrow({ where: { id: orgId }, select: { timezone: true } });
    // Revenue months begin in the ORG's zone. Expenses keep the server-local
    // boundary they always had: a separate variable so neither silently moves the other.
    const revenueStart = startOfMonthIn(now, org.timezone);
    const expenseStart = new Date(now.getFullYear(), now.getMonth(), 1);

    // Invoice payments MTD
    const revenueResult = await prisma.payment.aggregate({
      _sum: { amount: true },
      where: {
        date: { gte: revenueStart },
        invoice: { orgId },
      },
    });
    const invoicesMTD = new Prisma.Decimal(revenueResult._sum.amount ?? 0);

    // Counter takings MTD: what customers were CHARGED (Sale.totAmt), never the
    // tenders (gross of change). Voided sales are not takings.
    const counterResult = await prisma.sale.aggregate({
      _sum: { totAmt: true },
      where: { orgId, voidedAt: null, soldAt: { gte: revenueStart } },
    });
    const counterMTD = new Prisma.Decimal(counterResult._sum.totAmt ?? 0);

    // Expenses MTD
    const expenseResult = await prisma.expense.aggregate({
      _sum: { amount: true },
      where: { orgId, date: { gte: expenseStart } },
    });
    const expensesMTD = Number(expenseResult._sum.amount || 0);

    // Outstanding
    const invoices = await prisma.invoice.findMany({
      where: { orgId, status: { in: ["SENT", "PARTIAL", "OVERDUE"] }, deletedAt: null },
      select: { total: true, amountPaid: true },
    });
    const outstanding = invoices.reduce(
      (sum, inv) => sum + (Number(inv.total) - Number(inv.amountPaid)),
      0
    );

    // Low stock count
    const lowStockCount = await prisma.stockLevel.count({
      where: {
        product: { orgId },
        qty: { lte: 10 },
      },
    });

    // Revenue trend (last 6 months)
    const trendData = [];
    for (let i = 5; i >= 0; i--) {
      const monthStart = new Date(now.getFullYear(), now.getMonth() - i, 1);
      const monthEnd = new Date(now.getFullYear(), now.getMonth() - i + 1, 1);
      const monthRevenue = await prisma.payment.aggregate({
        _sum: { amount: true },
        where: {
          date: { gte: monthStart, lt: monthEnd },
          invoice: { orgId },
        },
      });
      trendData.push({
        date: monthStart.toLocaleDateString("en-US", { month: "short", year: "2-digit" }),
        revenue: Number(monthRevenue._sum.amount || 0),
      });
    }

    // Recent invoices
    const recentInvoices = await prisma.invoice.findMany({
      where: { orgId, deletedAt: null },
      include: { customer: { select: { name: true } } },
      orderBy: { createdAt: "desc" },
      take: 5,
    });

    return NextResponse.json({
      revenueMTD: { invoices: invoicesMTD.toFixed(2), counter: counterMTD.toFixed(2) },
      expensesMTD,
      outstanding,
      lowStockCount,
      trendData,
      recentInvoices,
    });
  } catch (error) {
    return handleRouteError(error);
  }
}
