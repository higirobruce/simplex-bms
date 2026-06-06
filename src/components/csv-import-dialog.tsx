"use client";

import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { parseCsv } from "@/lib/csv";
import type { ImportSummary } from "@/lib/import";
import { Upload } from "lucide-react";

interface CsvImportDialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  /** API path that accepts `{ rows }`, e.g. `/api/acme/products/import`. */
  endpoint: string;
  /** Comma-separated column list shown in the hint. */
  columns: string;
  /** Template CSV contents (header + one example row). */
  template: string;
  templateName: string;
  /** Called after a successful import (e.g. to invalidate the list query). */
  onImported?: (summary: ImportSummary) => void;
}

export function CsvImportDialog({
  open,
  onClose,
  title,
  endpoint,
  columns,
  template,
  templateName,
  onImported,
}: CsvImportDialogProps) {
  const [rows, setRows] = useState<Record<string, string>[]>([]);
  const [fileName, setFileName] = useState("");
  const [result, setResult] = useState<ImportSummary | null>(null);
  const [error, setError] = useState("");

  const importer = useMutation({
    mutationFn: (data: Record<string, string>[]) =>
      fetch(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ rows: data }),
      }).then(async (r) => {
        if (!r.ok) {
          const d = await r.json().catch(() => ({}));
          throw new Error(d.error || "Import failed");
        }
        return r.json() as Promise<ImportSummary>;
      }),
    onSuccess: (summary) => {
      setResult(summary);
      setRows([]);
      setFileName("");
      onImported?.(summary);
    },
    onError: (e: Error) => setError(e.message),
  });

  function reset() {
    setRows([]);
    setFileName("");
    setResult(null);
    setError("");
  }

  function close() {
    reset();
    onClose();
  }

  function onPickFile(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setResult(null);
    setError("");
    setFileName(file.name);
    const reader = new FileReader();
    reader.onload = () => {
      try {
        setRows(parseCsv(String(reader.result)));
      } catch {
        setError("Could not read that CSV file");
        setRows([]);
      }
    };
    reader.readAsText(file);
  }

  return (
    <Dialog open={open} onClose={close}>
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
      </DialogHeader>
      <div className="space-y-4">
        <p className="text-sm text-ink-soft">
          Upload a CSV with columns <span className="font-mono text-xs text-ink">{columns}</span>. Duplicate rows are skipped.
        </p>
        <a
          href={`data:text/csv;charset=utf-8,${encodeURIComponent(template)}`}
          download={templateName}
          className="inline-block text-sm text-gold underline-offset-4 hover:underline"
        >
          Download template
        </a>

        {error && (
          <div className="rounded-[var(--radius)] border-l-4 border-danger bg-danger-soft px-4 py-3 text-sm text-danger font-mono">
            {error}
          </div>
        )}

        {!result && (
          <>
            <label className="flex h-24 cursor-pointer flex-col items-center justify-center gap-2 rounded-[var(--radius)] border border-dashed border-line-strong bg-surface-2 text-ink-soft hover:border-gold">
              <Upload className="h-5 w-5" strokeWidth={1.75} />
              <span className="text-sm">{fileName || "Choose a .csv file"}</span>
              <input type="file" accept=".csv,text/csv" className="hidden" onChange={onPickFile} />
            </label>
            {rows.length > 0 && (
              <p className="text-sm text-ink">
                <span className="font-medium">{rows.length}</span> row{rows.length === 1 ? "" : "s"} ready to import.
              </p>
            )}
          </>
        )}

        {result && (
          <div className="rounded-[var(--radius)] border border-line bg-surface-2 p-4 text-sm space-y-2">
            <div className="flex gap-4">
              <span className="text-success font-medium">{result.created} created</span>
              <span className="text-ink-soft">{result.skipped} skipped</span>
              {result.errors > 0 && <span className="text-danger">{result.errors} errors</span>}
            </div>
            {result.errors > 0 && (
              <ul className="max-h-32 overflow-y-auto font-mono text-xs text-danger space-y-0.5">
                {result.results
                  .filter((r) => r.status === "error")
                  .slice(0, 20)
                  .map((r) => (
                    <li key={r.row}>Row {r.row}: {r.message}</li>
                  ))}
              </ul>
            )}
          </div>
        )}

        <div className="flex justify-end gap-3 pt-2">
          <Button type="button" variant="outline" onClick={close}>
            {result ? "Done" : "Cancel"}
          </Button>
          {!result && (
            <Button
              type="button"
              onClick={() => importer.mutate(rows)}
              disabled={rows.length === 0 || importer.isPending}
            >
              {importer.isPending ? "Importing…" : `Import ${rows.length || ""}`.trim()}
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
}
