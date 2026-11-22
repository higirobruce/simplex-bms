-- A parked outbound row is one the cloud refused with a status retrying can
-- never fix (422). It stays in the table (nothing is lost, the operator can
-- read `last_error`) but is no longer offered to the drain, so it cannot wedge
-- the rows behind it. NULL = not parked. A plain ADD COLUMN, no rebuild.
ALTER TABLE outbound ADD COLUMN parked_at TEXT;
