import { prisma } from "@/lib/prisma";
import { handleRouteError } from "@/lib/api";
import { getLaneContext } from "@/lib/lane-auth";
import { NextResponse } from "next/server";

// Sale ingest from the till's outbox. Status codes are load-bearing (see shift/route.ts):
//   401  bad/revoked token
//   422  a refusal retrying can NEVER fix: malformed body, unknown / foreign shift, a line whose
//        product cannot be resolved in this org (never existed or hard-deleted), a sale id or
//        invoice number that conflicts with a different sale. The till parks the row.
//   5xx  transient fault (database down, a lost race on a unique key). Anything not classified
//        here falls through to handleRouteError -> 500 and the till retries.
// laneId, orgId and the stock location come ONLY from the token; body fields of that kind are ignored.

const refuse = (message: string) => NextResponse.json({ error: message }, { status: 422 });

const HEX32 = /^[0-9a-f]{32}$/;
const MONEY = /^[0-9]{1,10}(\.[0-9]{1,2})?$/; // Decimal(12,2)
const QTY = /^[0-9]{1,9}(\.[0-9]{1,3})?$/; // Decimal(12,3)
const ISO_INSTANT = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})$/;
const TAX_TYPES = new Set(["A", "B", "C", "D"]);
const TENDER_METHODS = new Set(["CASH", "CREDIT", "CASH_CREDIT", "BANK_CHECK", "CARD", "MOBILE_MONEY", "OTHER"]);

type Tax = "A" | "B" | "C" | "D";

function date(v: unknown): Date | null {
  if (typeof v !== "string" || !ISO_INSTANT.test(v)) return null;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? null : d;
}
const money = (v: unknown): string | null => (typeof v === "string" && MONEY.test(v) ? v : null);
const qtyStr = (v: unknown): string | null => (typeof v === "string" && QTY.test(v) ? v : null);
const isObj = (v: unknown): v is Record<string, any> => !!v && typeof v === "object" && !Array.isArray(v);
const isInt = (v: unknown, min: number): v is number =>
  Number.isInteger(v) && (v as number) >= min && (v as number) <= 2147483647;

// Exact decimal addition on strings, scale 3, so merged quantities never touch a float.
function addQty(a: string, b: string): string {
  const mil = (s: string) => {
    const [i, f = ""] = s.split(".");
    return BigInt(i) * BigInt(1000) + BigInt(f.padEnd(3, "0"));
  };
  const t = mil(a) + mil(b);
  return `${t / BigInt(1000)}.${(t % BigInt(1000)).toString().padStart(3, "0")}`;
}

export async function POST(req: Request) {
  try {
    const { laneId, orgId, locationId } = await getLaneContext(req);

    let body: any;
    try {
      body = await req.json();
    } catch {
      return refuse("Body must be JSON");
    }
    if (!isObj(body)) return refuse("Body must be an object");

    // The till's cloud_payload names the sale id `saleId`; `id` is accepted as an alias.
    const saleId = body.saleId ?? body.id;
    if (typeof saleId !== "string" || !HEX32.test(saleId)) return refuse("saleId must be 32 lowercase hex characters");
    if (typeof body.shiftId !== "string" || !body.shiftId) return refuse("shiftId is required");
    if (!isInt(body.invcNo, 1)) return refuse("invcNo must be a positive integer");
    const invcNo: number = body.invcNo;
    const totAmt = money(body.totAmt);
    const totTaxblAmt = money(body.totTaxblAmt);
    const totTaxAmt = money(body.totTaxAmt);
    if (!totAmt || !totTaxblAmt || !totTaxAmt) {
      return refuse("totAmt, totTaxblAmt and totTaxAmt must be decimal strings with at most 2 places");
    }
    // soldAt is the till's own record. The till's payload calls it createdAt.
    const soldAt = date(body.soldAt ?? body.createdAt);
    if (!soldAt) return refuse("soldAt must be an ISO date with an explicit offset");
    let voidedAt: Date | null = null;
    if (body.voidedAt !== undefined && body.voidedAt !== null) {
      voidedAt = date(body.voidedAt);
      if (!voidedAt) return refuse("voidedAt must be an ISO date");
    }
    const buyerTin = body.buyerTin ?? null;
    if (buyerTin !== null && typeof buyerTin !== "string") return refuse("buyerTin must be a string");
    // rcptNo stays null until the receipt phase; accept it flat or inside `receipt`.
    const rawRcpt = body.rcptNo ?? (isObj(body.receipt) ? body.receipt.rcptNo : undefined) ?? null;
    if (rawRcpt !== null && !isInt(rawRcpt, 0)) return refuse("rcptNo must be an integer");

    if (!Array.isArray(body.lines) || body.lines.length === 0) return refuse("lines must be a non-empty array");
    if (!Array.isArray(body.tenders)) return refuse("tenders must be an array");

    const seqs = new Set<number>();
    const lines: { itemSeq: number; productId?: string; itemCd?: string; qty: string; prc: string; taxTyCd: Tax; dcAmt: string }[] = [];
    for (const l of body.lines) {
      if (!isObj(l)) return refuse("each line must be an object");
      if (!isInt(l.itemSeq, 0) || seqs.has(l.itemSeq)) return refuse("itemSeq must be a unique integer per line");
      seqs.add(l.itemSeq);
      const qty = qtyStr(l.qty);
      if (!qty || !(Number(qty) > 0)) return refuse("qty must be a positive decimal string with at most 3 places");
      const prc = money(l.prc);
      if (!prc) return refuse("prc must be a decimal string with at most 2 places");
      if (typeof l.taxTyCd !== "string" || !TAX_TYPES.has(l.taxTyCd)) return refuse("taxTyCd must be A, B, C or D");
      // The till's dcAmt is the line's discount in money (whole francs, exact string). Absent = none.
      const dcAmt = l.dcAmt === undefined || l.dcAmt === null ? "0" : money(l.dcAmt);
      if (dcAmt === null) return refuse("dcAmt must be a decimal string with at most 2 places");
      const hasPid = l.productId !== undefined && l.productId !== null;
      if (hasPid && (typeof l.productId !== "string" || !l.productId)) return refuse("productId must be a string");
      if (!hasPid && (typeof l.itemCd !== "string" || !l.itemCd)) return refuse("each line needs a productId or an itemCd");
      lines.push({
        itemSeq: l.itemSeq,
        qty,
        prc,
        taxTyCd: l.taxTyCd as Tax,
        dcAmt,
        productId: hasPid ? l.productId : undefined,
        itemCd: hasPid ? undefined : l.itemCd,
      });
    }

    const tenders: { pmtTyCd: any; amount: string; providerRef: string | null; confirmed: boolean }[] = [];
    for (const t of body.tenders) {
      if (!isObj(t)) return refuse("each tender must be an object");
      if (typeof t.pmtTyCd !== "string" || !TENDER_METHODS.has(t.pmtTyCd)) return refuse("unknown tender pmtTyCd");
      const amount = money(t.amount);
      if (!amount) return refuse("tender amount must be a decimal string with at most 2 places");
      if (t.providerRef != null && typeof t.providerRef !== "string") return refuse("providerRef must be a string");
      if (t.confirmed != null && typeof t.confirmed !== "boolean") return refuse("confirmed must be a boolean");
      tenders.push({ pmtTyCd: t.pmtTyCd, amount, providerRef: t.providerRef ?? null, confirmed: t.confirmed ?? false });
    }

    // The shift must exist and belong to this lane (the outbox is ordered, so it normally does).
    const shift = await prisma.shift.findUnique({ where: { id: body.shiftId }, select: { laneId: true, orgId: true } });
    if (!shift || shift.laneId !== laneId || shift.orgId !== orgId) return refuse("Unknown shift for this lane");

    // The sale id and (lane, invcNo) must name the same sale, and the id must not be another org's.
    const [byId, byInvc] = await Promise.all([
      prisma.sale.findUnique({ where: { id: saleId }, select: { laneId: true, orgId: true, invcNo: true } }),
      prisma.sale.findUnique({ where: { laneId_invcNo: { laneId, invcNo } }, select: { id: true } }),
    ]);
    if (byId && (byId.laneId !== laneId || byId.orgId !== orgId || byId.invcNo !== invcNo)) {
      return refuse("Sale id belongs to a different sale");
    }
    if (byInvc && byInvc.id !== saleId) return refuse("invcNo already belongs to a different sale");

    // Resolve every line to a product of THIS org. Soft-deleted products are accepted (the sale
    // happened); a missing row would be an FK violation (500, retried forever), so refuse it here.
    const ids = Array.from(new Set(lines.flatMap((l) => (l.productId ? [l.productId] : []))));
    const skus = Array.from(new Set(lines.flatMap((l) => (l.itemCd ? [l.itemCd] : []))));
    const byPid = ids.length
      ? await prisma.product.findMany({ where: { orgId, id: { in: ids } }, select: { id: true } })
      : [];
    // Fallback for rows written before the till carried product ids.
    const bySku = skus.length
      ? await prisma.product.findMany({ where: { orgId, sku: { in: skus } }, select: { id: true, sku: true } })
      : [];
    const pidOk = new Set(byPid.map((p) => p.id));
    const skuMap = new Map(bySku.map((p) => [p.sku, p.id]));
    const resolved: { itemSeq: number; productId: string; qty: string; prc: string; taxTyCd: Tax; dcAmt: string }[] = [];
    for (const l of lines) {
      const productId = l.productId ? (pidOk.has(l.productId) ? l.productId : undefined) : skuMap.get(l.itemCd!);
      if (!productId) return refuse(`Line ${l.itemSeq} names a product that does not exist in this shop`);
      resolved.push({ itemSeq: l.itemSeq, productId, qty: l.qty, prc: l.prc, taxTyCd: l.taxTyCd, dcAmt: l.dcAmt });
    }

    // The partial unique index is on (referenceId, productId) for SALE, so two lines of one
    // product would collide: write one movement per product with the summed quantity.
    const perProduct = new Map<string, string>();
    for (const l of resolved) perProduct.set(l.productId, addQty(perProduct.get(l.productId) ?? "0", l.qty));

    // A summed quantity that cannot fit Decimal(12,3) is a permanent refusal. As a 500 it would
    // wedge every later sale from the lane (ordered outbox, retried forever).
    const QTY_MAX = BigInt(999999999999); // 999999999.999 in thousandths
    const thousandths = (s: string) => {
      const [i, f = ""] = s.split(".");
      return BigInt(i) * BigInt(1000) + BigInt(f.padEnd(3, "0"));
    };
    for (const q of Array.from(perProduct.values())) {
      if (thousandths(q) > QTY_MAX) return refuse("Total quantity of one product is too large");
    }

    await prisma.$transaction(async (tx) => {
      await tx.sale.upsert({
        where: { id: saleId },
        create: {
          id: saleId, shiftId: body.shiftId, laneId, orgId, invcNo, rcptNo: rawRcpt,
          totAmt, totTaxblAmt, totTaxAmt, buyerTin, voidedAt, soldAt,
        },
        // A replay changes nothing except a void that arrives later.
        update: voidedAt ? { voidedAt } : {},
        select: { id: true },
      });
      await tx.saleLine.createMany({
        data: resolved.map(({ dcAmt, ...l }) => ({
          ...l, saleId, orgId,
          // dcRt is derived from dcAmt on the till, so the exact money amount is what is kept.
          // Lines then reconcile with the header: qty*prc - discountValue.
          ...(Number(dcAmt) > 0 ? { discountKind: "AMOUNT" as const, discountValue: dcAmt } : {}),
        })),
        skipDuplicates: true,
      });
      await tx.tender.deleteMany({ where: { saleId, orgId } });
      await tx.tender.createMany({ data: tenders.map((t) => ({ ...t, saleId, orgId })) });
      // One movement per product. A replay inserts nothing (ON CONFLICT DO NOTHING on the partial
      // unique index), and StockLevel is decremented ONLY when a row was actually inserted, so a
      // replay moves no stock. A till sale already happened, so on-hand may go negative.
      for (const [productId, qty] of Array.from(perProduct.entries())) {
        const made = await tx.stockMovement.createMany({
          data: [{ type: "SALE" as const, productId, qty, fromLocationId: locationId, referenceId: saleId, orgId }],
          skipDuplicates: true,
        });
        if (made.count === 1) {
          await tx.stockLevel.upsert({
            where: { productId_locationId: { productId, locationId } },
            update: { qty: { decrement: qty } },
            create: { productId, locationId, qty: "-" + qty },
          });
        }
      }
      // Never backwards: an out-of-order replay of an older sale cannot lower the counter.
      await tx.lane.updateMany({ where: { id: laneId, lastInvcNo: { lt: invcNo } }, data: { lastInvcNo: invcNo } });
    });
    return NextResponse.json({ id: saleId });
  } catch (error) {
    return handleRouteError(error);
  }
}
