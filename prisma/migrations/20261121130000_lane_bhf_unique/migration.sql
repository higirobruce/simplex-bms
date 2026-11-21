-- One lane per (bhfId, org): the till's invoice sequence is per till database,
-- so two lanes sharing a bhfId would issue colliding invoice numbers at the
-- VSDC. bhfId stays nullable; Postgres treats NULLs as distinct in a unique
-- index, so any number of unconfigured lanes remain legal.
-- CreateIndex
CREATE UNIQUE INDEX "Lane_bhfId_orgId_key" ON "Lane"("bhfId", "orgId");
