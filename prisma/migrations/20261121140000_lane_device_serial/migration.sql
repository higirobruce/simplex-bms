-- Correction of 20261121130000_lane_bhf_unique, which rested on a false premise.
-- RRA issues bhfId per BRANCH, so every lane of a shop shares it; the VSDC keeps
-- its invoice counters per DEVICE (vsdc.txt 3.3.1.1, InitDevice). The real
-- per-till identity is the device serial, so bhfId goes back to non-unique and
-- dvcSrlNo is added. NULLs are distinct in a Postgres unique index, so several
-- unconfigured lanes remain legal.
DROP INDEX "Lane_bhfId_orgId_key";

ALTER TABLE "Lane" ADD COLUMN "dvcSrlNo" TEXT;

CREATE UNIQUE INDEX "Lane_dvcSrlNo_orgId_key" ON "Lane"("dvcSrlNo", "orgId");
