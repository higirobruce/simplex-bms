"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useMemo } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { useTenant } from "@/lib/hooks";
import { PageHeader } from "@/components/page-header";
import {
  WarehouseTree,
  buildTree,
  type WarehouseNode,
  type TreeNode,
} from "@/components/warehouse-tree";
import { Warehouse, Plus, Search, Pencil, Trash2 } from "lucide-react";

function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, {
    ...opts,
    headers: { "Content-Type": "application/json", ...opts?.headers },
  }).then(async (r) => {
    if (!r.ok) {
      const d = await r.json();
      throw new Error(d.error || "Request failed");
    }
    return r.json();
  });
}

function collectDescendantIds(tree: TreeNode[], rootId: string): Set<string> {
  const out = new Set<string>();
  const walk = (node: TreeNode, include: boolean) => {
    if (include) out.add(node.id);
    node.children.forEach((c) => walk(c, include || node.id === rootId));
  };
  tree.forEach((n) => walk(n, n.id === rootId));
  // Also include the root itself
  out.add(rootId);
  return out;
}

export default function InventoryPage() {
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [selectedLocId, setSelectedLocId] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const [showLocForm, setShowLocForm] = useState(false);
  const [editingLoc, setEditingLoc] = useState<TreeNode | null>(null);
  const [parentForNew, setParentForNew] = useState<string | null>(null);
  const [deletingLoc, setDeletingLoc] = useState<TreeNode | null>(null);
  const [locForm, setLocForm] = useState({ name: "", code: "", address: "" });

  const [showAdjust, setShowAdjust] = useState(false);
  const [adjustForm, setAdjustForm] = useState({
    productId: "",
    locationId: "",
    qty: "",
    type: "ADJUSTMENT",
    reason: "",
  });

  const { data: locations = [] } = useQuery<WarehouseNode[]>({
    queryKey: ["locations", slug],
    queryFn: () => fetch(`/api/${slug}/locations`).then((r) => r.json()),
    enabled: !!slug,
  });

  const { data: productsResp } = useQuery({
    queryKey: ["products", slug, "all"],
    queryFn: () =>
      fetch(`/api/${slug}/products?limit=500`).then((r) => r.json()),
    enabled: !!slug,
  });
  const products = Array.isArray(productsResp)
    ? productsResp
    : productsResp?.data ?? [];

  const tree = useMemo(() => buildTree(locations), [locations]);

  const stockRows = useMemo(() => {
    if (!Array.isArray(products)) return [];
    return products.flatMap((p: any) =>
      (p.stockLevels || []).map((sl: any) => ({
        productId: p.id,
        sku: p.sku,
        name: p.name,
        locationId: sl.locationId,
        locationName:
          locations.find((l) => l.id === sl.locationId)?.name || "Unknown",
        qty: sl.qty,
        reorderLevel: p.reorderLevel,
      }))
    );
  }, [products, locations]);

  const visibleIds = useMemo(() => {
    if (!selectedLocId) return null;
    return collectDescendantIds(tree, selectedLocId);
  }, [tree, selectedLocId]);

  const filtered = stockRows.filter((r) => {
    if (visibleIds && !visibleIds.has(r.locationId)) return false;
    if (
      search &&
      !r.name.toLowerCase().includes(search.toLowerCase()) &&
      !r.sku.toLowerCase().includes(search.toLowerCase())
    )
      return false;
    return true;
  });

  // Mutations
  const createLoc = useMutation({
    mutationFn: (data: any) =>
      apiCall(`/api/${slug}/locations`, {
        method: "POST",
        body: JSON.stringify(data),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["locations", slug] });
      setShowLocForm(false);
      setLocForm({ name: "", code: "", address: "" });
      setParentForNew(null);
      toast.success("Warehouse created");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const updateLoc = useMutation({
    mutationFn: ({ id, data }: { id: string; data: any }) =>
      apiCall(`/api/${slug}/locations/${id}`, {
        method: "PATCH",
        body: JSON.stringify(data),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["locations", slug] });
      setEditingLoc(null);
      toast.success("Warehouse updated");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const removeLoc = useMutation({
    mutationFn: (id: string) =>
      apiCall(`/api/${slug}/locations/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["locations", slug] });
      setDeletingLoc(null);
      toast.success("Warehouse deleted");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const adjust = useMutation({
    mutationFn: (data: any) =>
      apiCall(`/api/${slug}/stock/adjust`, {
        method: "POST",
        body: JSON.stringify(data),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["products", slug] });
      queryClient.invalidateQueries({ queryKey: ["locations", slug] });
      setShowAdjust(false);
      setAdjustForm({
        productId: "",
        locationId: "",
        qty: "",
        type: "ADJUSTMENT",
        reason: "",
      });
      toast.success("Stock adjusted");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const totalUnits = stockRows.reduce((s, r) => s + r.qty, 0);
  const lowStockCount = stockRows.filter(
    (r) => r.qty <= r.reorderLevel
  ).length;

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Workspace"
        title="Inventory"
        description="Stock by warehouse — every piece accounted for."
        actions={
          <>
            <Button
              variant="outline"
              onClick={() => {
                setParentForNew(null);
                setLocForm({ name: "", code: "", address: "" });
                setShowLocForm(true);
              }}
            >
              <Plus className="h-4 w-4" strokeWidth={2} /> New warehouse
            </Button>
            <Button onClick={() => setShowAdjust(true)} size="lg">
              <Plus className="h-4 w-4" strokeWidth={2} /> Adjust stock
            </Button>
          </>
        }
      />

      {/* Summary */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-5">
          <p className="text-[0.65rem] tracking-[0.2em] uppercase text-ink-mute font-medium">
            Total units
          </p>
          <p className="font-display text-3xl font-bold uppercase tracking-tight mt-2 text-ink tabular">
            {totalUnits.toLocaleString()}
          </p>
        </div>
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-5">
          <p className="text-[0.65rem] tracking-[0.2em] uppercase text-ink-mute font-medium">
            Warehouses
          </p>
          <p className="font-display text-3xl font-bold uppercase tracking-tight mt-2 text-ink tabular">
            {locations.length}
          </p>
        </div>
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-5">
          <p className="text-[0.65rem] tracking-[0.2em] uppercase text-ink-mute font-medium">
            Low stock items
          </p>
          <p className="font-display text-3xl font-bold uppercase tracking-tight mt-2 text-warning tabular">
            {lowStockCount}
          </p>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-[280px_1fr] gap-5">
        {/* Tree */}
        <aside className="rounded-[var(--radius-lg)] border border-line bg-surface p-4 h-fit">
          <div className="px-2 mb-3 flex items-center justify-between">
            <h2 className="text-[0.65rem] tracking-[0.2em] uppercase text-ink-mute font-medium">
              Locations
            </h2>
            <button
              onClick={() => setSelectedLocId(null)}
              className={
                "text-[0.65rem] tracking-[0.18em] uppercase " +
                (selectedLocId ? "text-accent hover:underline" : "text-ink-mute")
              }
            >
              All
            </button>
          </div>
          {tree.length === 0 ? (
            <div className="px-2 py-6 text-sm text-ink-mute italic">
              No warehouses yet — add your first.
            </div>
          ) : (
            <WarehouseTree
              nodes={tree}
              selectedId={selectedLocId}
              onSelect={setSelectedLocId}
              onAddChild={(parentId) => {
                setParentForNew(parentId);
                setLocForm({ name: "", code: "", address: "" });
                setShowLocForm(true);
              }}
              onEdit={(node) => {
                setEditingLoc(node);
                setLocForm({
                  name: node.name,
                  code: node.code || "",
                  address: "",
                });
              }}
              onDelete={(node) => setDeletingLoc(node)}
            />
          )}
        </aside>

        {/* Stock table */}
        <div>
          <div className="relative mb-4 max-w-md">
            <Search
              className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute"
              strokeWidth={1.75}
            />
            <Input
              placeholder="Search by product name or SKU…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="pl-11 rounded-full"
            />
          </div>

          {filtered.length === 0 ? (
            <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line">
              <Warehouse
                className="mx-auto h-10 w-10 text-ink-mute mb-4"
                strokeWidth={1.5}
              />
              <p className="font-display font-bold text-xl uppercase tracking-tight text-ink">
                No stock here
              </p>
              <p className="text-sm text-ink-soft mt-1">
                {selectedLocId
                  ? "This warehouse holds nothing yet."
                  : "Receive goods or adjust stock to begin."}
              </p>
            </div>
          ) : (
            <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>SKU</TableHead>
                    <TableHead>Product</TableHead>
                    <TableHead>Location</TableHead>
                    <TableHead className="text-right">Qty</TableHead>
                    <TableHead className="text-right">Reorder</TableHead>
                    <TableHead>Status</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filtered.map((row, i) => (
                    <TableRow
                      key={`${row.productId}-${row.locationId}-${i}`}
                    >
                      <TableCell className="font-mono text-xs text-ink-soft tabular">
                        {row.sku}
                      </TableCell>
                      <TableCell className="font-medium text-ink">
                        {row.name}
                      </TableCell>
                      <TableCell className="text-ink-soft">
                        {row.locationName}
                      </TableCell>
                      <TableCell className="text-right font-medium tabular">
                        {row.qty}
                      </TableCell>
                      <TableCell className="text-right text-ink-soft tabular">
                        {row.reorderLevel}
                      </TableCell>
                      <TableCell>
                        {row.qty <= row.reorderLevel ? (
                          <Badge variant="warning">Low stock</Badge>
                        ) : (
                          <Badge variant="success">In stock</Badge>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </div>

      {/* Create / Edit warehouse */}
      <Dialog
        open={showLocForm || !!editingLoc}
        onClose={() => {
          setShowLocForm(false);
          setEditingLoc(null);
          setParentForNew(null);
        }}
      >
        <DialogHeader>
          <DialogTitle>
            {editingLoc
              ? "Edit warehouse"
              : parentForNew
              ? "Add sub-location"
              : "New warehouse"}
          </DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (editingLoc) {
              updateLoc.mutate({
                id: editingLoc.id,
                data: {
                  name: locForm.name,
                  code: locForm.code || null,
                  address: locForm.address || null,
                },
              });
            } else {
              createLoc.mutate({
                name: locForm.name,
                code: locForm.code || null,
                address: locForm.address || null,
                parentId: parentForNew,
              });
            }
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Name</Label>
            <Input
              value={locForm.name}
              onChange={(e) =>
                setLocForm({ ...locForm, name: e.target.value })
              }
              placeholder="Main warehouse"
              required
            />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Code</Label>
              <Input
                value={locForm.code}
                onChange={(e) =>
                  setLocForm({ ...locForm, code: e.target.value })
                }
                placeholder="WH-01"
              />
            </div>
            <div className="space-y-2">
              <Label>Address</Label>
              <Input
                value={locForm.address}
                onChange={(e) =>
                  setLocForm({ ...locForm, address: e.target.value })
                }
              />
            </div>
          </div>
          {parentForNew && (
            <p className="text-xs text-ink-soft">
              This will be created under the selected parent.
            </p>
          )}
          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setShowLocForm(false);
                setEditingLoc(null);
                setParentForNew(null);
              }}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={createLoc.isPending || updateLoc.isPending}
            >
              {editingLoc ? "Save changes" : "Create"}
            </Button>
          </div>
        </form>
      </Dialog>

      <ConfirmDialog
        open={!!deletingLoc}
        onClose={() => setDeletingLoc(null)}
        onConfirm={() => deletingLoc && removeLoc.mutate(deletingLoc.id)}
        title="Delete warehouse"
        description={`Delete "${deletingLoc?.name}"? Sub-locations and stock must be empty first.`}
        confirmLabel="Delete"
        variant="destructive"
        loading={removeLoc.isPending}
      />

      {/* Stock adjust */}
      <Dialog open={showAdjust} onClose={() => setShowAdjust(false)}>
        <DialogHeader>
          <DialogTitle>Adjust stock</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            adjust.mutate({
              ...adjustForm,
              qty: parseInt(adjustForm.qty),
            });
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Product</Label>
            <Select
              value={adjustForm.productId}
              onChange={(e) =>
                setAdjustForm({ ...adjustForm, productId: e.target.value })
              }
              required
            >
              <option value="">Select product…</option>
              {products.map((p: any) => (
                <option key={p.id} value={p.id}>
                  {p.sku} — {p.name}
                </option>
              ))}
            </Select>
          </div>
          <div className="space-y-2">
            <Label>Location</Label>
            <Select
              value={adjustForm.locationId}
              onChange={(e) =>
                setAdjustForm({ ...adjustForm, locationId: e.target.value })
              }
              required
            >
              <option value="">Select location…</option>
              {locations.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                  {l.code ? ` (${l.code})` : ""}
                </option>
              ))}
            </Select>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Qty (+ add, − remove)</Label>
              <Input
                type="number"
                value={adjustForm.qty}
                onChange={(e) =>
                  setAdjustForm({ ...adjustForm, qty: e.target.value })
                }
                placeholder="e.g. 10 or -5"
                required
              />
            </div>
            <div className="space-y-2">
              <Label>Type</Label>
              <Select
                value={adjustForm.type}
                onChange={(e) =>
                  setAdjustForm({ ...adjustForm, type: e.target.value })
                }
              >
                <option value="RECEIPT">Receipt</option>
                <option value="ISSUE">Issue</option>
                <option value="ADJUSTMENT">Adjustment</option>
                <option value="TRANSFER">Transfer</option>
                <option value="RETURN">Return</option>
              </Select>
            </div>
          </div>
          <div className="space-y-2">
            <Label>Reason (optional)</Label>
            <Input
              value={adjustForm.reason}
              onChange={(e) =>
                setAdjustForm({ ...adjustForm, reason: e.target.value })
              }
              placeholder="e.g. recount, damaged"
            />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setShowAdjust(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={adjust.isPending}>
              {adjust.isPending ? "Adjusting…" : "Adjust"}
            </Button>
          </div>
        </form>
      </Dialog>
    </div>
  );
}
