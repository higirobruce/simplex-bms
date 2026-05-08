"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { useTenant } from "@/lib/hooks";
import { PageHeader } from "@/components/page-header";
import { Warehouse, Plus, Search } from "lucide-react";

function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, { ...opts, headers: { "Content-Type": "application/json", ...opts?.headers } }).then(async (r) => {
    if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Request failed"); }
    return r.json();
  });
}

export default function StockPage() {
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [showAdjust, setShowAdjust] = useState(false);
  const [form, setForm] = useState({ productId: "", locationId: "", qty: "", type: "ADJUSTMENT" as string, reason: "" });

  const { data: productsResp, isLoading } = useQuery({
    queryKey: ["products", slug, "all"],
    queryFn: () =>
      fetch(`/api/${slug}/products?limit=500`).then((r) => r.json()),
    enabled: !!slug,
  });

  const products = Array.isArray(productsResp)
    ? productsResp
    : productsResp?.data ?? [];

  const { data: locations } = useQuery({
    queryKey: ["locations", slug],
    queryFn: () => fetch(`/api/${slug}/locations`).then((r) => r.json()),
    enabled: !!slug,
  });

  const adjust = useMutation({
    mutationFn: (data: any) => apiCall(`/api/${slug}/stock/adjust`, { method: "POST", body: JSON.stringify(data) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["products", slug] });
      setShowAdjust(false);
      setForm({ productId: "", locationId: "", qty: "", type: "ADJUSTMENT", reason: "" });
      toast.success("Stock adjusted");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  // Build flat stock rows from products
  const stockRows = products.flatMap((p: any) =>
    (p.stockLevels || []).map((sl: any) => ({
      productId: p.id,
      sku: p.sku,
      name: p.name,
      locationId: sl.locationId,
      locationName: sl.location?.name || locations?.find((l: any) => l.id === sl.locationId)?.name || "Unknown",
      qty: sl.qty,
      reorderLevel: p.reorderLevel,
    }))
  ) || [];

  const filtered = stockRows.filter((r: any) =>
    r.name.toLowerCase().includes(search.toLowerCase()) || r.sku.toLowerCase().includes(search.toLowerCase())
  );

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Workspace"
        title="Inventory"
        description="Stock levels by location — every piece accounted for."
        actions={
          <Button onClick={() => setShowAdjust(true)} size="lg">
            <Plus className="h-4 w-4" strokeWidth={2} /> Adjust Stock
          </Button>
        }
      />
      <div className="relative mb-6 max-w-md">
        <Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute" strokeWidth={1.75} />
        <Input placeholder="Search by product name or SKU…" value={search} onChange={(e) => setSearch(e.target.value)} className="pl-11 rounded-full" />
      </div>
      {isLoading ? <div className="text-center py-16 text-ink-mute font-serif italic">Loading…</div> : filtered.length === 0 ? (
        <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line"><Warehouse className="mx-auto h-10 w-10 text-ink-mute mb-4" strokeWidth={1.5} /><p className="font-serif italic text-lg text-ink">No stock data yet</p></div>
      ) : (
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden">
          <Table>
            <TableHeader><TableRow>
              <TableHead>SKU</TableHead><TableHead>Product</TableHead><TableHead>Location</TableHead>
              <TableHead className="text-right">Qty</TableHead><TableHead className="text-right">Reorder Level</TableHead><TableHead>Status</TableHead>
            </TableRow></TableHeader>
            <TableBody>
              {filtered.map((row: any, i: number) => (
                <TableRow key={`${row.productId}-${row.locationId}-${i}`}>
                  <TableCell className="font-mono text-xs text-ink-soft tabular">{row.sku}</TableCell>
                  <TableCell className="font-medium text-ink">{row.name}</TableCell>
                  <TableCell className="text-ink-soft">{row.locationName}</TableCell>
                  <TableCell className="text-right font-medium tabular">{row.qty}</TableCell>
                  <TableCell className="text-right text-ink-soft tabular">{row.reorderLevel}</TableCell>
                  <TableCell>
                    {row.qty <= row.reorderLevel ? <Badge variant="warning">Low stock</Badge> : <Badge variant="success">In stock</Badge>}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <Dialog open={showAdjust} onClose={() => setShowAdjust(false)}>
        <DialogHeader><DialogTitle>Adjust Stock</DialogTitle></DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); adjust.mutate({ ...form, qty: parseInt(form.qty) }); }} className="space-y-4">
          <div className="space-y-2"><Label>Product</Label>
            <Select value={form.productId} onChange={(e) => setForm({ ...form, productId: e.target.value })} required>
              <option value="">Select product...</option>
              {products.map((p: any) => <option key={p.id} value={p.id}>{p.sku} — {p.name}</option>)}
            </Select>
          </div>
          <div className="space-y-2"><Label>Location</Label>
            <Select value={form.locationId} onChange={(e) => setForm({ ...form, locationId: e.target.value })} required>
              <option value="">Select location...</option>
              {locations?.map((l: any) => <option key={l.id} value={l.id}>{l.name}</option>)}
            </Select>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>Quantity (+ to add, - to remove)</Label>
              <Input type="number" value={form.qty} onChange={(e) => setForm({ ...form, qty: e.target.value })} placeholder="e.g. 10 or -5" required />
            </div>
            <div className="space-y-2"><Label>Type</Label>
              <Select value={form.type} onChange={(e) => setForm({ ...form, type: e.target.value })}>
                <option value="RECEIPT">Receipt</option><option value="ISSUE">Issue</option><option value="ADJUSTMENT">Adjustment</option>
                <option value="TRANSFER">Transfer</option><option value="RETURN">Return</option>
              </Select>
            </div>
          </div>
          <div className="space-y-2"><Label>Reason (optional)</Label>
            <Input value={form.reason} onChange={(e) => setForm({ ...form, reason: e.target.value })} placeholder="e.g. Damaged goods, recount..." />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setShowAdjust(false)}>Cancel</Button>
            <Button type="submit" disabled={adjust.isPending}>{adjust.isPending ? "Adjusting..." : "Adjust Stock"}</Button>
          </div>
        </form>
      </Dialog>
    </div>
  );
}
