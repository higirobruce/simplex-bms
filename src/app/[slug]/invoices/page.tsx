"use client";

import { useQuery } from "@tanstack/react-query";
import { useState, useEffect } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Select } from "@/components/ui/select";
import { useTenant, useDebounce } from "@/lib/hooks";
import { Pagination } from "@/components/ui/pagination";
import { PageHeader } from "@/components/page-header";
import { formatDate } from "@/lib/utils";
import { useMoney } from "@/lib/currency";
import { Plus, Search, FileText } from "lucide-react";

const statusVariant: Record<string, "success" | "warning" | "destructive" | "secondary" | "outline" | "info"> = {
  PAID: "success",
  SENT: "info",
  PARTIAL: "warning",
  OVERDUE: "destructive",
  DRAFT: "outline",
  VOID: "secondary",
};

export default function InvoicesPage() {
  const fmt = useMoney();
  const { slug } = useTenant();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState("");
  const debouncedSearch = useDebounce(search, 300);

  useEffect(() => { setPage(1); }, [debouncedSearch, statusFilter]);

  const { data: invoices, isLoading } = useQuery({
    queryKey: ["invoices", slug, page, debouncedSearch, statusFilter],
    queryFn: () => fetch(`/api/${slug}/invoices?page=${page}&limit=20&search=${debouncedSearch}&status=${statusFilter}`).then((r) => r.json()),
    enabled: !!slug,
  });

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Commerce"
        title="Invoices"
        description="The ledger of your house — issued, settled, and pending."
        actions={
          <Link href={`/${slug}/invoices/new`}>
            <Button size="lg">
              <Plus className="h-4 w-4" strokeWidth={2} /> New Invoice
            </Button>
          </Link>
        }
      />

      <div className="flex gap-3 mb-6">
        <div className="relative flex-1 max-w-md">
          <Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute" strokeWidth={1.75} />
          <Input
            placeholder="Search by invoice number or customer…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-11 rounded-full"
          />
        </div>
        <Select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)} className="w-44 rounded-full">
          <option value="">All Statuses</option>
          <option value="DRAFT">Draft</option>
          <option value="SENT">Sent</option>
          <option value="PAID">Paid</option>
          <option value="PARTIAL">Partial</option>
          <option value="OVERDUE">Overdue</option>
          <option value="VOID">Void</option>
        </Select>
      </div>

      {isLoading ? (
        <div className="text-center py-16 font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">Loading invoices…</div>
      ) : invoices?.data?.length === 0 ? (
        <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line">
          <FileText className="mx-auto h-10 w-10 text-ink-mute mb-4" strokeWidth={1.5} />
          <p className="font-display font-bold text-xl uppercase tracking-tight text-ink">No invoices yet</p>
        </div>
      ) : (
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Invoice #</TableHead>
                <TableHead>Customer</TableHead>
                <TableHead>Date</TableHead>
                <TableHead>Due Date</TableHead>
                <TableHead className="text-right">Total</TableHead>
                <TableHead className="text-right">Paid</TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {invoices?.data?.map((invoice: any) => (
                <TableRow key={invoice.id}>
                  <TableCell>
                    <Link
                      href={`/${slug}/invoices/${invoice.id}`}
                      className="font-medium text-accent hover:underline underline-offset-4 tabular"
                    >
                      {invoice.invoiceNo}
                    </Link>
                  </TableCell>
                  <TableCell className="text-ink">{invoice.customer?.name || "—"}</TableCell>
                  <TableCell className="text-ink-soft tabular">{formatDate(invoice.createdAt)}</TableCell>
                  <TableCell className="text-ink-soft tabular">{formatDate(invoice.dueDate)}</TableCell>
                  <TableCell className="text-right font-medium tabular">
                    {fmt(invoice.total)}
                  </TableCell>
                  <TableCell className="text-right tabular text-ink-soft">
                    {fmt(invoice.amountPaid)}
                  </TableCell>
                  <TableCell>
                    <Badge variant={statusVariant[invoice.status] || "secondary"}>
                      {invoice.status?.toLowerCase()}
                    </Badge>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <Pagination page={page} totalPages={invoices?.meta?.pages || 1} onPageChange={setPage} />
    </div>
  );
}
