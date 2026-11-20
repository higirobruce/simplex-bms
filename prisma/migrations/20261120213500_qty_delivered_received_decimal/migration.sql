-- AlterTable
ALTER TABLE "PurchaseOrderLine" ALTER COLUMN "qtyReceived" SET DEFAULT 0,
ALTER COLUMN "qtyReceived" SET DATA TYPE DECIMAL(12,3);

-- AlterTable
ALTER TABLE "SalesOrderLine" ALTER COLUMN "qtyDelivered" SET DEFAULT 0,
ALTER COLUMN "qtyDelivered" SET DATA TYPE DECIMAL(12,3);

