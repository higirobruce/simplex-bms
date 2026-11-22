-- Correction of 20261121130000_lane_bhf_unique, which rested on a false premise.
-- RRA issues bhfId per BRANCH, so every lane of a shop shares it; the VSDC keeps
-- its invoice counters per DEVICE (vsdc.txt 3.3.1.1, InitDevice). The real
-- per-till identity is the device serial, so bhfId goes back to non-unique and
-- dvcSrlNo is added. NULLs are distinct in a Postgres unique index, so several
-- unconfigured lanes remain legal.
-- IF EXISTS because 20261121130000_lane_bhf_unique cannot apply to a database
-- that holds real lane data: RRA issues bhfId per BRANCH, so every shop with
-- two lanes legitimately has duplicates and that CREATE UNIQUE INDEX fails.
-- Where it failed, this index was never created, so an unconditional DROP
-- would fail in turn and wedge the whole chain. Resolved 2026-11-22.
DROP INDEX IF EXISTS "Lane_bhfId_orgId_key";

ALTER TABLE "Lane" ADD COLUMN "dvcSrlNo" TEXT;

CREATE UNIQUE INDEX "Lane_dvcSrlNo_orgId_key" ON "Lane"("dvcSrlNo", "orgId");
