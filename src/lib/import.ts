import type { ZodType } from "zod";

export interface ImportRowResult {
  row: number;
  status: "created" | "skipped" | "error";
  message?: string;
}

export interface ImportSummary {
  created: number;
  skipped: number;
  errors: number;
  total: number;
  results: ImportRowResult[];
}

// Runs a per-row import: validates each row independently, calls `create`, and
// reports an outcome per row so one bad row never fails the whole file.
// A unique-constraint violation (P2002) is reported as "skipped".
export async function runRowImport<T>(
  rows: unknown[],
  rowSchema: ZodType<T>,
  create: (data: T) => Promise<void>,
  duplicateMessage: (data: T) => string = () => "Already exists"
): Promise<ImportSummary> {
  const results: ImportRowResult[] = [];
  let created = 0;
  let skipped = 0;

  for (let i = 0; i < rows.length; i++) {
    const rowNo = i + 1;
    const parsed = rowSchema.safeParse(rows[i]);
    if (!parsed.success) {
      results.push({ row: rowNo, status: "error", message: parsed.error.issues[0].message });
      continue;
    }
    try {
      await create(parsed.data);
      created++;
      results.push({ row: rowNo, status: "created" });
    } catch (e: any) {
      if (e?.code === "P2002") {
        skipped++;
        results.push({ row: rowNo, status: "skipped", message: duplicateMessage(parsed.data) });
      } else {
        results.push({ row: rowNo, status: "error", message: "Could not import" });
      }
    }
  }

  const errors = results.filter((r) => r.status === "error").length;
  return { created, skipped, errors, total: results.length, results };
}
