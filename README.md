# Simplex BMS

Multi-tenant business management system (Next.js 14, Prisma, PostgreSQL, NextAuth). Originally built for
building-materials distributors, now being extended into a supermarket retail system for the Rwandan
market: weighed goods, EBM tax codes (A/B/C/D), promotions, batches with best-before dates, and a
lane/shift/sale/tender model ahead of a Tauri till app.

## Setup

```bash
npm install
# first create .env with DATABASE_URL (your dev database), NEXTAUTH_SECRET and NEXTAUTH_URL
npx prisma migrate deploy   # apply migrations
npm run db:seed             # demo data (building materials)
npm run db:seed:grocery     # demo data (a Rwandan supermarket)
npm run dev
```

## Tests

The suite uses a separate PostgreSQL database, `bms_test`. The test database is configured by a
gitignored `.env.test` file, which you must create yourself:

```bash
# .env.test
DATABASE_URL="postgresql://<user>@localhost:5432/bms_test"
NEXTAUTH_SECRET="any-test-secret"
NEXTAUTH_URL="http://localhost:3000"
```

`DATABASE_URL` must point at a database named `bms_test`; `tests/helpers/guard.ts` refuses anything else,
because the tests truncate every table. Then run `npm test`: the global setup creates `bms_test` if absent
and applies migrations (`prisma migrate deploy`).

Without `.env.test` the setup only warns and skips the database; the database-free tests (skin, qty, CSV
template) still run, and every database test fails loudly. A green run on a fresh clone with no `.env.test`
therefore does not mean the schema tests ran.

## Migrations

- Do not use `prisma migrate dev` / `migrate reset` / `migrate diff` against a database you care about:
  they need a shadow database and can offer to reset your data. Hand-write the SQL, mirror
  `prisma/schema.prisma` exactly, and apply with `prisma migrate deploy`. Constraints Prisma cannot express
  (CHECK, EXCLUDE) are appended by hand, as in `20261121090000_promotions`.
- **The quantity widening is one-way.** Migrations `20261120212241_qty_decimal` and
  `20261120213500_qty_delivered_received_decimal` changed ten quantity columns from `INTEGER` to
  `DECIMAL(12,3)` so weighed goods can have fractional quantities. Prisma has no down-migrations, and
  narrowing back to `int4` would truncate real data. To roll back, restore from a backup.

## Quantities and money

Never use `+` on a Prisma `Decimal` (`new Prisma.Decimal("10") + 5` yields the string `"105"`). Use the
helpers in `src/lib/qty.ts` (`toQty`, `addQty`, `isNegative`, `formatQty`, `lineAmount`).

## Importing products

Products > Import accepts a CSV; the dialog's template lists every column, including `barcode`, `plu`,
`taxTyCd` (`A`, `B`, `C` or `D`; blank means `B`, 18% VAT, so set `A` for exempt goods), `department`,
`isWeighed` (`true`/`yes`/`y`/`1`, case-insensitive; blank means no), `packSize` and `shelfLifeDays`.
