-- CreateEnum
CREATE TYPE "TenderMethod" AS ENUM ('CASH', 'CREDIT', 'CASH_CREDIT', 'BANK_CHECK', 'CARD', 'MOBILE_MONEY');

-- CreateTable
CREATE TABLE "Lane" (
    "id" TEXT NOT NULL,
    "code" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "locationId" TEXT NOT NULL,
    "orgId" TEXT NOT NULL,
    "mrc" TEXT,
    "bhfId" TEXT,
    "lastInvcNo" INTEGER NOT NULL DEFAULT 0,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "deletedAt" TIMESTAMP(3),

    CONSTRAINT "Lane_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Shift" (
    "id" TEXT NOT NULL,
    "laneId" TEXT NOT NULL,
    "openedById" TEXT NOT NULL,
    "openedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "closedAt" TIMESTAMP(3),
    "openingFloat" DECIMAL(12,2) NOT NULL,
    "declaredCash" DECIMAL(12,2),
    "orgId" TEXT NOT NULL,

    CONSTRAINT "Shift_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Sale" (
    "id" TEXT NOT NULL,
    "shiftId" TEXT NOT NULL,
    "laneId" TEXT NOT NULL,
    "invcNo" INTEGER NOT NULL,
    "rcptNo" INTEGER,
    "totAmt" DECIMAL(12,2) NOT NULL,
    "totTaxblAmt" DECIMAL(12,2) NOT NULL,
    "totTaxAmt" DECIMAL(12,2) NOT NULL,
    "buyerTin" TEXT,
    "discountKind" "DiscountKind",
    "discountValue" DECIMAL(12,2),
    "voidedAt" TIMESTAMP(3),
    "orgId" TEXT NOT NULL,
    "soldAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "Sale_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "SaleLine" (
    "id" TEXT NOT NULL,
    "saleId" TEXT NOT NULL,
    "productId" TEXT NOT NULL,
    "itemSeq" INTEGER NOT NULL,
    "qty" DECIMAL(12,3) NOT NULL,
    "prc" DECIMAL(12,2) NOT NULL,
    "taxTyCd" "TaxType" NOT NULL,
    "discountKind" "DiscountKind",
    "discountValue" DECIMAL(12,2),
    "orgId" TEXT NOT NULL,

    CONSTRAINT "SaleLine_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Tender" (
    "id" TEXT NOT NULL,
    "saleId" TEXT NOT NULL,
    "pmtTyCd" "TenderMethod" NOT NULL,
    "amount" DECIMAL(12,2) NOT NULL,
    "providerRef" TEXT,
    "confirmed" BOOLEAN NOT NULL DEFAULT false,
    "orgId" TEXT NOT NULL,

    CONSTRAINT "Tender_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "Lane_code_orgId_key" ON "Lane"("code", "orgId");

-- CreateIndex
CREATE INDEX "Shift_orgId_laneId_idx" ON "Shift"("orgId", "laneId");

-- CreateIndex
CREATE INDEX "Sale_orgId_soldAt_idx" ON "Sale"("orgId", "soldAt");

-- CreateIndex
CREATE UNIQUE INDEX "Sale_laneId_invcNo_key" ON "Sale"("laneId", "invcNo");

-- CreateIndex
CREATE UNIQUE INDEX "SaleLine_saleId_itemSeq_key" ON "SaleLine"("saleId", "itemSeq");

-- CreateIndex
CREATE INDEX "Tender_saleId_idx" ON "Tender"("saleId");

-- AddForeignKey
ALTER TABLE "Lane" ADD CONSTRAINT "Lane_locationId_fkey" FOREIGN KEY ("locationId") REFERENCES "Location"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "Shift" ADD CONSTRAINT "Shift_laneId_fkey" FOREIGN KEY ("laneId") REFERENCES "Lane"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "Sale" ADD CONSTRAINT "Sale_shiftId_fkey" FOREIGN KEY ("shiftId") REFERENCES "Shift"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "Sale" ADD CONSTRAINT "Sale_laneId_fkey" FOREIGN KEY ("laneId") REFERENCES "Lane"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "SaleLine" ADD CONSTRAINT "SaleLine_saleId_fkey" FOREIGN KEY ("saleId") REFERENCES "Sale"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "SaleLine" ADD CONSTRAINT "SaleLine_productId_fkey" FOREIGN KEY ("productId") REFERENCES "Product"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "Tender" ADD CONSTRAINT "Tender_saleId_fkey" FOREIGN KEY ("saleId") REFERENCES "Sale"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
