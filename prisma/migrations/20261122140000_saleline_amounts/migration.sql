-- SaleLine keeps the till's rounded line amounts so lines reconcile with the header.
-- Nullable: rows written before this migration have none.
ALTER TABLE "SaleLine" ADD COLUMN "splyAmt" DECIMAL(12,2),
ADD COLUMN "taxblAmt" DECIMAL(12,2);

-- Tender gets the till's per-tender seq so ingest is idempotent. Existing rows are numbered
-- per sale in insertion order (cuid ids sort by creation) before the unique is added.
ALTER TABLE "Tender" ADD COLUMN "seq" INTEGER;
UPDATE "Tender" t SET "seq" = r.n
FROM (SELECT "id", ROW_NUMBER() OVER (PARTITION BY "saleId" ORDER BY "id") - 1 AS n FROM "Tender") r
WHERE t."id" = r."id";
ALTER TABLE "Tender" ALTER COLUMN "seq" SET NOT NULL;
CREATE UNIQUE INDEX "Tender_saleId_seq_key" ON "Tender"("saleId", "seq");
