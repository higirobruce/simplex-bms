-- NOTE (irreversible): Postgres cannot remove a value from an enum. Adding 'OTHER' to
-- "TenderMethod" is a one-way change; roll back by restoring a backup, never by reversing it.
-- Adds RRA VSDC payment code 07 (spec v1.0.4 section 4.10). The new value is not used in this
-- migration, so it is safe inside Prisma's transaction on PostgreSQL 12+.
ALTER TYPE "TenderMethod" ADD VALUE 'OTHER';
