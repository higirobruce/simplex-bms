-- Till/back-office sync: cashier role, till PIN, per-lane device token.
ALTER TYPE "Role" ADD VALUE 'CASHIER';

ALTER TABLE "User" ADD COLUMN "pinHash" TEXT;

CREATE TABLE "LaneToken" (
    "id" TEXT NOT NULL,
    "laneId" TEXT NOT NULL,
    "tokenHash" TEXT NOT NULL,
    "label" TEXT,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "lastUsedAt" TIMESTAMP(3),
    "revokedAt" TIMESTAMP(3),
    "orgId" TEXT NOT NULL,

    CONSTRAINT "LaneToken_pkey" PRIMARY KEY ("id")
);

CREATE UNIQUE INDEX "LaneToken_tokenHash_key" ON "LaneToken"("tokenHash");
CREATE INDEX "LaneToken_laneId_idx" ON "LaneToken"("laneId");

ALTER TABLE "LaneToken" ADD CONSTRAINT "LaneToken_laneId_fkey" FOREIGN KEY ("laneId") REFERENCES "Lane"("id") ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE "LaneToken" ADD CONSTRAINT "LaneToken_orgId_fkey" FOREIGN KEY ("orgId") REFERENCES "Org"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- One SALE movement per (sale, product). Partial on purpose: GRN and delivery-note
-- approval also write referenceId, and a GRN may hold two movements of one product
-- in different batches, so a global unique would break goods receipt.
CREATE UNIQUE INDEX "StockMovement_sale_ref_product_key"
  ON "StockMovement" ("referenceId", "productId")
  WHERE "type" = 'SALE';
