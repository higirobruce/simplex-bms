import type { Prisma } from "@prisma/client";

type Tx = Prisma.TransactionClient;

/**
 * Atomic, self-healing per-org per-doc-kind sequence.
 *
 * Generates the next document number (e.g. "SO-0042") for a given org and kind.
 * If pre-existing documents exist (e.g. seeded data) with a higher number than
 * the current sequence row, the sequence is bumped to MAX(existing) + 1 so new
 * numbers never collide.
 */
export async function nextDocNo(
  tx: Tx,
  orgId: string,
  kind: "SO" | "DN" | "PO" | "GRN",
  table: "SalesOrder" | "DeliveryNote" | "PurchaseOrder" | "GoodsReceiptNote",
  prefix: string,
  pad = 4
): Promise<string> {
  const rows = await tx.$queryRawUnsafe<{ lastSeq: number }[]>(
    `WITH max_existing AS (
       SELECT COALESCE(
         MAX(CAST(SUBSTRING("${
           table === "SalesOrder" || table === "PurchaseOrder"
             ? "orderNo"
             : "noteNo"
         }" FROM ${prefix.length + 2}) AS INTEGER)),
         0
       ) AS n
       FROM "${table}"
       WHERE "orgId" = $1
         AND "${
           table === "SalesOrder" || table === "PurchaseOrder"
             ? "orderNo"
             : "noteNo"
         }" ~ '^${prefix}-[0-9]+$'
     )
     INSERT INTO "DocSequence" ("orgId", "kind", "lastSeq")
     SELECT $1, $2, n + 1 FROM max_existing
     ON CONFLICT ("orgId", "kind") DO UPDATE
       SET "lastSeq" = GREATEST(
         "DocSequence"."lastSeq" + 1,
         (SELECT n + 1 FROM max_existing)
       )
     RETURNING "lastSeq"`,
    orgId,
    kind
  );
  return `${prefix}-${String(rows[0].lastSeq).padStart(pad, "0")}`;
}
