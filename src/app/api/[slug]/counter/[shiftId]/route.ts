import { prisma } from "@/lib/prisma";
import { getTenantContext, handleRouteError } from "@/lib/api";
import { SALES_CAP } from "@/lib/counter-view";
import { NextResponse } from "next/server";

// One shift's individual sales, with lines and tenders. The tenant comes from the
// session, never the URL. A shift in another org is a 404, indistinguishable from
// one that does not exist. Every money value is the STORED decimal rendered as a
// string: line amounts are splyAmt/taxblAmt as the till wrote them, never qty * prc.
export async function GET(_req: Request, { params }: { params: { shiftId: string } }) {
  try {
    const { orgId } = await getTenantContext();

    const shift = await prisma.shift.findFirst({
      where: { id: params.shiftId, orgId },
      select: {
        id: true,
        sales: {
          where: { orgId },
          take: SALES_CAP,
          orderBy: [{ soldAt: "asc" }, { invcNo: "asc" }],
          select: {
            id: true, invcNo: true, rcptNo: true, soldAt: true, voidedAt: true,
            totAmt: true, totTaxblAmt: true, totTaxAmt: true, buyerTin: true,
            lines: {
              where: { orgId },
              orderBy: { itemSeq: "asc" },
              select: {
                itemSeq: true, qty: true, prc: true, taxTyCd: true, splyAmt: true, taxblAmt: true,
                product: { select: { name: true } },
              },
            },
            tenders: {
              where: { orgId },
              orderBy: { seq: "asc" },
              select: { seq: true, pmtTyCd: true, amount: true },
            },
          },
        },
      },
    });
    if (!shift) return NextResponse.json({ error: "Shift not found" }, { status: 404 });
    const salesTotal = await prisma.sale.count({ where: { shiftId: shift.id, orgId } });

    const money = (d: { toFixed(n: number): string } | null) => (d === null ? null : d.toFixed(2));
    return NextResponse.json({
      shiftId: shift.id,
      // Not the fiscal copy: RRA holds that. Product names are the CURRENT names,
      // which may differ from what was printed on the receipt at the time.
      record: {
        kind: "back-office",
        fiscalCopy: false,
        note: "Back-office record, not the fiscal receipt. Product names are current names and may differ from the printed receipt.",
      },
      salesTotal,
      truncated: salesTotal > shift.sales.length,
      sales: shift.sales.map((s) => ({
        id: s.id,
        invcNo: s.invcNo,
        rcptNo: s.rcptNo,
        soldAt: s.soldAt.toISOString(),
        voidedAt: s.voidedAt?.toISOString() ?? null,
        totAmt: money(s.totAmt),
        totTaxblAmt: money(s.totTaxblAmt),
        totTaxAmt: money(s.totTaxAmt),
        buyerTin: s.buyerTin,
        lines: s.lines.map((l) => ({
          itemSeq: l.itemSeq,
          name: l.product.name,
          qty: l.qty.toFixed(3),
          prc: l.prc.toFixed(2),
          taxTyCd: l.taxTyCd,
          // Stored by the till; null on rows that predate the columns. Never derived.
          splyAmt: money(l.splyAmt),
          taxblAmt: money(l.taxblAmt),
        })),
        tenders: s.tenders.map((t) => ({ seq: t.seq, method: t.pmtTyCd, amount: money(t.amount) })),
      })),
    });
  } catch (error) {
    return handleRouteError(error);
  }
}
