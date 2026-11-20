-- CreateEnum
CREATE TYPE "TaxType" AS ENUM ('A', 'B', 'C', 'D');

-- AlterTable
ALTER TABLE "Product" ADD COLUMN     "barcode" TEXT,
ADD COLUMN     "department" TEXT NOT NULL DEFAULT 'General',
ADD COLUMN     "isWeighed" BOOLEAN NOT NULL DEFAULT false,
ADD COLUMN     "packSize" TEXT,
ADD COLUMN     "plu" TEXT,
ADD COLUMN     "shelfLifeDays" INTEGER,
ADD COLUMN     "taxTyCd" "TaxType" NOT NULL DEFAULT 'B';

-- CreateIndex
CREATE INDEX "Product_plu_orgId_idx" ON "Product"("plu", "orgId");

-- CreateIndex
CREATE UNIQUE INDEX "Product_barcode_orgId_key" ON "Product"("barcode", "orgId");

