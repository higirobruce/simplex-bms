"use client";

import { useQuery } from "@tanstack/react-query";
import { useState, useEffect } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Pagination } from "@/components/ui/pagination";
import { PageHeader } from "@/components/page-header";
import { useTenant, useDebounce } from "@/lib/hooks";
import { formatDate } from "@/lib/utils";
import { useMoney } from "@/lib/currency";
import { Plus, Search, PackageOpen } from "lucide-react";

const statusVariant: Record<string, any> = {
  DRAFT: "secondary",
  CONFIRMED: "info",
  RECEIVED: "success",
  CANCELLED: "destructive",
};

export default function ProcurementPage() {
  const fmt = useMoney();
  const { slug } = useTenant();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState("");
  const debouncedSearch = useDebounce(search, 300);

  useEffect(() => {
    setPage(1);
  }, [debouncedSearch, statusFilter]);

  const { data: orders, isLoading } = useQuery({
    queryKey: ["purchase-orders", slug, page, debouncedSearch, statusFilter],
    queryFn: () =>
      fetch(
        `/api/${slug}/purchase-orders?page=${page}&limit=20&search=${debouncedSearch}&status=${statusFilter}`
      ).then((r) => r.json()),
    enabled: !!slug,
  });

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Procurement"
        title="Purchase orders"
        description="Source goods from your vendors and record them on arrival."
        actions={
          <Link href={`/${slug}/procurement/new`}>
            <Button size="lg">
              <Plus className="h-4 w-4" strokeWidth={2} /> New purchase order
            </Button>
          </Link>
        }
      />

      <div className="flex gap-3 mb-6">
        <div className="relative flex-1 max-w-md">
          <Search
            className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute"
            strokeWidth={1.75}
          />
          <Input
            placeholder="Search by PO # or vendor…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-11 rounded-full"
          />
        </div>
        <Select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value)}
          className="w-44 rounded-full"
        >
          <option value="">All Statuses</option>
          <option value="DRAFT">Draft</option>
          <option value="CONFIRMED">Confirmed</option>
          <option value="RECEIVED">Received</option>
          <option value="CANCELLED">Cancelled</option>
        </Select>
      </div>

      {isLoading ? (
        <div className="text-center py-16 font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">
          Loading purchase orders…
        </div>
      ) : orders?.data?.length === 0 ? (
        <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line">
          <PackageOpen
            className="mx-auto h-10 w-10 text-ink-mute mb-4"
            strokeWidth={1.5}
          />
          <p className="font-display font-bold text-xl uppercase tracking-tight text-ink">
            No purchase orders yet
          </p>
        </div>
      ) : (
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>PO #</TableHead>
                <TableHead>Vendor</TableHead>
                <TableHead>Date</TableHead>
                <TableHead className="text-right">Lines</TableHead>
                <TableHead className="text-right">Total</TableHead>
                <TableHead>Receipts</TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {orders?.data?.map((o: any) => (
                <TableRow key={o.id}>
                  <TableCell>
                    <Link
                      href={`/${slug}/procurement/${o.id}`}
                      className="font-medium text-accent hover:underline underline-offset-4 tabular"
                    >
                      {o.orderNo}
                    </Link>
                  </TableCell>
                  <TableCell className="text-ink">
                    {o.vendor?.name || "—"}
                  </TableCell>
                  <TableCell className="text-ink-soft tabular">
                    {formatDate(o.orderDate)}
                  </TableCell>
                  <TableCell className="text-right tabular text-ink-soft">
                    {o._count?.lines ?? 0}
                  </TableCell>
                  <TableCell className="text-right font-medium tabular">
                    {fmt(o.total)}
                  </TableCell>
                  <TableCell className="text-ink-soft tabular">
                    {o._count?.receipts ?? 0}
                  </TableCell>
                  <TableCell>
                    <Badge variant={statusVariant[o.status] || "secondary"}>
                      {o.status?.toLowerCase()}
                    </Badge>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <Pagination
        page={page}
        totalPages={orders?.meta?.pages || 1}
        onPageChange={setPage}
      />
    </div>
  );
}
