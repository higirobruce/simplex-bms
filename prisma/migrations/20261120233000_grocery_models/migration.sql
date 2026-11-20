-- CreateEnum
CREATE TYPE "EbmSyncState" AS ENUM ('PENDING', 'SENT', 'ACKNOWLEDGED', 'FAILED');

-- CreateEnum
CREATE TYPE "WriteOffReason" AS ENUM ('EXPIRY', 'DAMAGE', 'THEFT', 'MARKDOWN');

-- AlterTable
ALTER TABLE "StockMovement" ADD COLUMN     "batchId" TEXT,
ADD COLUMN     "ebmSyncState" "EbmSyncState" NOT NULL DEFAULT 'PENDING';

-- CreateTable
CREATE TABLE "Batch" (
    "id" TEXT NOT NULL,
    "productId" TEXT NOT NULL,
    "lot" TEXT NOT NULL,
    "receivedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "bestBefore" TIMESTAMP(3),
    "orgId" TEXT NOT NULL,

    CONSTRAINT "Batch_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "StockWriteOff" (
    "id" TEXT NOT NULL,
    "productId" TEXT NOT NULL,
    "locationId" TEXT NOT NULL,
    "batchId" TEXT,
    "qty" DECIMAL(12,3) NOT NULL,
    "reason" "WriteOffReason" NOT NULL,
    "note" TEXT,
    "orgId" TEXT NOT NULL,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "StockWriteOff_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE INDEX "Batch_orgId_productId_idx" ON "Batch"("orgId", "productId");

-- CreateIndex
CREATE INDEX "Batch_orgId_bestBefore_idx" ON "Batch"("orgId", "bestBefore");

-- CreateIndex
CREATE INDEX "StockWriteOff_orgId_createdAt_idx" ON "StockWriteOff"("orgId", "createdAt");

-- CreateIndex
CREATE INDEX "StockWriteOff_orgId_productId_idx" ON "StockWriteOff"("orgId", "productId");

-- CreateIndex
CREATE INDEX "StockMovement_batchId_idx" ON "StockMovement"("batchId");

-- AddForeignKey
ALTER TABLE "StockMovement" ADD CONSTRAINT "StockMovement_batchId_fkey" FOREIGN KEY ("batchId") REFERENCES "Batch"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "Batch" ADD CONSTRAINT "Batch_productId_fkey" FOREIGN KEY ("productId") REFERENCES "Product"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "StockWriteOff" ADD CONSTRAINT "StockWriteOff_productId_fkey" FOREIGN KEY ("productId") REFERENCES "Product"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "StockWriteOff" ADD CONSTRAINT "StockWriteOff_locationId_fkey" FOREIGN KEY ("locationId") REFERENCES "Location"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "StockWriteOff" ADD CONSTRAINT "StockWriteOff_batchId_fkey" FOREIGN KEY ("batchId") REFERENCES "Batch"("id") ON DELETE SET NULL ON UPDATE CASCADE;

