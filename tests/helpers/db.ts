import { PrismaClient } from "@prisma/client";
import { assertTestDatabaseUrl } from "./guard";

// Test-only client. Never import @/lib/prisma from tests: that one is bound to the dev database.
// Hard guard: refuse to run against anything but bms_test (e.g. if .env.test is missing and
// DATABASE_URL fell back to the dev database), because resetDb truncates everything.
const url = process.env.DATABASE_URL ?? "";
assertTestDatabaseUrl(url);

export const prisma = new PrismaClient({ datasourceUrl: url });

/** Truncate every application table (discovered, not hardcoded), keeping the schema. */
export async function resetDb(): Promise<void> {
  const rows = await prisma.$queryRaw<{ tablename: string }[]>`
    select table_name as tablename from information_schema.tables
    where table_schema = 'public' and table_type = 'BASE TABLE'
      and table_name <> '_prisma_migrations'`;
  if (rows.length === 0) return;
  const list = rows.map((r) => `"public"."${r.tablename.replace(/"/g, '""')}"`).join(", ");
  await prisma.$executeRawUnsafe(`truncate table ${list} restart identity cascade`);
}

export async function seedOrg(): Promise<{
  orgId: string;
  otherOrgId: string;
  userId: string;
  productId: string;
  locationId: string;
}> {
  const tag = Math.random().toString(36).slice(2, 8);
  const org = await prisma.org.create({ data: { slug: `test-${tag}`, name: "Test Supermarket" } });
  const other = await prisma.org.create({ data: { slug: `other-${tag}`, name: "Other Supermarket" } });
  const user = await prisma.user.create({
    data: { email: `owner-${tag}@test.local`, password: "x", name: "Test Owner", role: "ADMIN", orgId: org.id },
  });
  const location = await prisma.location.create({
    data: { name: "Main Store", code: "MAIN", orgId: org.id },
  });
  const product = await prisma.product.create({
    data: { sku: `SKU-${tag}`, name: "Test Product", unitPrice: "1000.00", orgId: org.id },
  });
  return {
    orgId: org.id,
    otherOrgId: other.id,
    userId: user.id,
    productId: product.id,
    locationId: location.id,
  };
}
