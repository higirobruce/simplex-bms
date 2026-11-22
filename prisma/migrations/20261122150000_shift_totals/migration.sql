-- Shift keeps the till's own sale count and gross, sent with the shift close, so the back office
-- can compare them with what it holds. Nullable: shifts closed before this have neither, and
-- both null means completeness unknown (a zero saleCount is a real value: nothing sold).
ALTER TABLE "Shift" ADD COLUMN "saleCount" INTEGER,
ADD COLUMN "gross" DECIMAL(12,2);
