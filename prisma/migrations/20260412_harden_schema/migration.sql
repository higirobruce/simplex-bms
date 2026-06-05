-- 1. Create enums
CREATE TYPE "InvoiceStatus" AS ENUM ('DRAFT', 'SENT', 'PARTIAL', 'OVERDUE', 'PAID', 'VOID');
CREATE TYPE "EntityStatus" AS ENUM ('ACTIVE', 'INACTIVE');
CREATE TYPE "MovementType" AS ENUM ('RECEIPT', 'ISSUE', 'TRANSFER', 'ADJUSTMENT', 'RETURN');
CREATE TYPE "PaymentMethod" AS ENUM ('CASH', 'MOBILE_MONEY', 'BANK', 'CHECK', 'OTHER');

-- 2. Normalize existing data before enum conversion
UPDATE "Invoice" SET "status" = 'DRAFT' WHERE "status" = 'draft';
UPDATE "Invoice" SET "status" = 'SENT' WHERE "status" = 'sent';
UPDATE "Invoice" SET "status" = 'PARTIAL' WHERE "status" = 'partial';
UPDATE "Invoice" SET "status" = 'OVERDUE' WHERE "status" = 'overdue';
UPDATE "Invoice" SET "status" = 'PAID' WHERE "status" = 'paid';

UPDATE "Customer" SET "status" = 'ACTIVE' WHERE "status" = 'active';
UPDATE "Customer" SET "status" = 'INACTIVE' WHERE "status" = 'inactive';

UPDATE "Vendor" SET "status" = 'ACTIVE' WHERE "status" = 'active';
UPDATE "Vendor" SET "status" = 'INACTIVE' WHERE "status" = 'inactive';

-- Normalize stock movement types
UPDATE "StockMovement" SET "type" = 'RECEIPT' WHERE "type" IN ('purchase', 'PURCHASE', 'receipt');
UPDATE "StockMovement" SET "type" = 'ISSUE' WHERE "type" IN ('sale', 'SALE', 'issue');
UPDATE "StockMovement" SET "type" = 'ADJUSTMENT' WHERE "type" IN ('adjustment', 'write-off', 'WRITE_OFF');
UPDATE "StockMovement" SET "type" = 'TRANSFER' WHERE "type" IN ('transfer', 'TRANSFER');
UPDATE "StockMovement" SET "type" = 'RETURN' WHERE "type" IN ('return', 'RETURN');

-- Normalize payment methods
UPDATE "Payment" SET "method" = 'CASH' WHERE "method" = 'cash';
UPDATE "Payment" SET "method" = 'MOBILE_MONEY' WHERE "method" = 'mobile_money';
UPDATE "Payment" SET "method" = 'BANK' WHERE "method" = 'bank';
UPDATE "Payment" SET "method" = 'CHECK' WHERE "method" = 'check';

-- 3. Convert status/type/method columns to enums
-- (drop text default first; an existing text default cannot be auto-cast to the enum type)
ALTER TABLE "Invoice" ALTER COLUMN "status" DROP DEFAULT;
ALTER TABLE "Invoice" ALTER COLUMN "status" TYPE "InvoiceStatus" USING "status"::"InvoiceStatus";
ALTER TABLE "Invoice" ALTER COLUMN "status" SET DEFAULT 'DRAFT';

ALTER TABLE "Customer" ALTER COLUMN "status" DROP DEFAULT;
ALTER TABLE "Customer" ALTER COLUMN "status" TYPE "EntityStatus" USING "status"::"EntityStatus";
ALTER TABLE "Customer" ALTER COLUMN "status" SET DEFAULT 'ACTIVE';

ALTER TABLE "Vendor" ALTER COLUMN "status" DROP DEFAULT;
ALTER TABLE "Vendor" ALTER COLUMN "status" TYPE "EntityStatus" USING "status"::"EntityStatus";
ALTER TABLE "Vendor" ALTER COLUMN "status" SET DEFAULT 'ACTIVE';

ALTER TABLE "StockMovement" ALTER COLUMN "type" TYPE "MovementType" USING "type"::"MovementType";

ALTER TABLE "Payment" ALTER COLUMN "method" TYPE "PaymentMethod" USING "method"::"PaymentMethod";

-- 4. Add updatedAt columns with default for existing rows
ALTER TABLE "Product" ADD COLUMN "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE "Customer" ADD COLUMN "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE "Vendor" ADD COLUMN "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE "Invoice" ADD COLUMN "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE "Payment" ADD COLUMN "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE "Expense" ADD COLUMN "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP;

-- 5. Standardize InvoiceLineItem.unitPrice precision
ALTER TABLE "InvoiceLineItem" ALTER COLUMN "unitPrice" TYPE DECIMAL(12, 2);

-- 6. Standardize Product price precision
ALTER TABLE "Product" ALTER COLUMN "unitPrice" TYPE DECIMAL(12, 2);
ALTER TABLE "Product" ALTER COLUMN "costPrice" TYPE DECIMAL(12, 2);

-- 7. Create InvoiceSequence table
CREATE TABLE "InvoiceSequence" (
    "orgId" TEXT NOT NULL,
    "lastSeq" INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT "InvoiceSequence_pkey" PRIMARY KEY ("orgId")
);
ALTER TABLE "InvoiceSequence" ADD CONSTRAINT "InvoiceSequence_orgId_fkey" FOREIGN KEY ("orgId") REFERENCES "Org"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- Initialize sequence from existing invoice counts
INSERT INTO "InvoiceSequence" ("orgId", "lastSeq")
SELECT "orgId", COUNT(*) FROM "Invoice" GROUP BY "orgId";

-- 8. Add missing indexes
CREATE INDEX "Invoice_orgId_status_idx" ON "Invoice"("orgId", "status");
CREATE INDEX "Invoice_orgId_dueDate_idx" ON "Invoice"("orgId", "dueDate");
CREATE INDEX "Customer_orgId_status_idx" ON "Customer"("orgId", "status");
CREATE INDEX "Vendor_orgId_status_idx" ON "Vendor"("orgId", "status");
CREATE INDEX "StockMovement_orgId_createdAt_idx" ON "StockMovement"("orgId", "createdAt");
