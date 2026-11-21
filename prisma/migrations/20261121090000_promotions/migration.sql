-- CreateEnum
CREATE TYPE "DiscountKind" AS ENUM ('PERCENT', 'AMOUNT');

-- CreateTable
CREATE TABLE "Promotion" (
    "id" TEXT NOT NULL,
    "productId" TEXT NOT NULL,
    "price" DECIMAL(12,2) NOT NULL,
    "startsAt" TIMESTAMP(3) NOT NULL,
    "endsAt" TIMESTAMP(3) NOT NULL,
    "orgId" TEXT NOT NULL,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "Promotion_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "PriceChange" (
    "id" TEXT NOT NULL,
    "productId" TEXT NOT NULL,
    "oldPrice" DECIMAL(12,2) NOT NULL,
    "newPrice" DECIMAL(12,2) NOT NULL,
    "changedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "changedById" TEXT NOT NULL,
    "orgId" TEXT NOT NULL,

    CONSTRAINT "PriceChange_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE INDEX "Promotion_orgId_startsAt_idx" ON "Promotion"("orgId", "startsAt");

-- CreateIndex
CREATE INDEX "PriceChange_orgId_productId_changedAt_idx" ON "PriceChange"("orgId", "productId", "changedAt");

-- AddForeignKey
ALTER TABLE "Promotion" ADD CONSTRAINT "Promotion_productId_fkey" FOREIGN KEY ("productId") REFERENCES "Product"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "PriceChange" ADD CONSTRAINT "PriceChange_productId_fkey" FOREIGN KEY ("productId") REFERENCES "Product"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- Hand-written: Prisma's schema language cannot express these two constraints.
-- tsrange (not tstzrange) because Prisma's DateTime is timestamp without time zone;
-- the timestamp->timestamptz cast is not IMMUTABLE and is rejected in index expressions.
-- tsrange is half-open [), so a promotion may start exactly when another ends.
CREATE EXTENSION IF NOT EXISTS btree_gist;
ALTER TABLE "Promotion" ADD CONSTRAINT "promotion_dates_ordered" CHECK ("endsAt" > "startsAt");
ALTER TABLE "Promotion" ADD CONSTRAINT "promotion_no_overlap" EXCLUDE USING gist ("productId" WITH =, tsrange("startsAt", "endsAt") WITH &&);
