"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { useTenant, useDebounce } from "@/lib/hooks";
import { Pagination } from "@/components/ui/pagination";
import { formatCurrency } from "@/lib/utils";
import { Plus, Search, Package, Pencil, Trash2 } from "lucide-react";

export default function ProductsPage() {
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const debouncedSearch = useDebounce(search, 300);
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<any>(null);
  const [deleting, setDeleting] = useState<any>(null);
  const [form, setForm] = useState({
    sku: "", name: "", unitPrice: "", costPrice: "", category: "General", reorderLevel: "10", description: "",
  });

  useEffect(() => { setPage(1); }, [debouncedSearch]);

  const { data: products, isLoading } = useQuery({
    queryKey: ["products", slug, page, debouncedSearch],
    queryFn: () => fetch(`/api/${slug}/products?page=${page}&limit=20&search=${debouncedSearch}`).then((r) => r.json()),
    enabled: !!slug,
  });

  const createProduct = useMutation({
    mutationFn: (data: any) =>
      fetch(`/api/${slug}/products`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(data),
      }).then(async (r) => {
        if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Failed"); }
        return r.json();
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["products", slug] });
      setShowAdd(false);
      setForm({ sku: "", name: "", unitPrice: "", costPrice: "", category: "General", reorderLevel: "10", description: "" });
      toast.success("Product created");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const updateProduct = useMutation({
    mutationFn: ({ id, data }: { id: string; data: any }) =>
      fetch(`/api/${slug}/products/${id}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(data),
      }).then(async (r) => {
        if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Failed"); }
        return r.json();
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["products", slug] });
      setEditing(null);
      toast.success("Product updated");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const deleteProduct = useMutation({
    mutationFn: (id: string) =>
      fetch(`/api/${slug}/products/${id}`, { method: "DELETE" }).then(async (r) => {
        if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Failed"); }
        return r.json();
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["products", slug] });
      setDeleting(null);
      toast.success("Product deleted");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    createProduct.mutate({
      ...form,
      unitPrice: parseFloat(form.unitPrice),
      costPrice: form.costPrice ? parseFloat(form.costPrice) : null,
      reorderLevel: parseInt(form.reorderLevel),
    });
  };

  const handleUpdate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!editing) return;
    updateProduct.mutate({
      id: editing.id,
      data: {
        name: editing.name,
        unitPrice: parseFloat(editing.unitPrice),
        costPrice: editing.costPrice ? parseFloat(editing.costPrice) : null,
        category: editing.category,
        reorderLevel: parseInt(editing.reorderLevel),
        description: editing.description || null,
      },
    });
  };

  const openEdit = (product: any) => {
    setEditing({
      id: product.id,
      name: product.name,
      unitPrice: String(product.unitPrice),
      costPrice: product.costPrice ? String(product.costPrice) : "",
      category: product.category,
      reorderLevel: String(product.reorderLevel),
      description: product.description || "",
    });
  };

  return (
    <div className="animate-fade-in">
      <div className="flex flex-col lg:flex-row justify-between items-start lg:items-end gap-6 mb-9 pb-7 border-b border-line">
        <div>
          <p className="text-[0.7rem] tracking-[0.22em] uppercase text-gold font-medium mb-3">
            Catalogue
          </p>
          <h1 className="font-serif text-5xl leading-tight text-ink">
            Products
          </h1>
          <p className="mt-3 text-ink-soft max-w-xl leading-relaxed">
            Curate your offering — pricing, inventory thresholds, and
            availability across the house.
          </p>
        </div>
        <Button onClick={() => setShowAdd(true)} size="lg">
          <Plus className="h-4 w-4" strokeWidth={2} /> Add Product
        </Button>
      </div>

      <div className="relative mb-6 max-w-md">
        <Search
          className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute"
          strokeWidth={1.75}
        />
        <Input
          placeholder="Search by name or SKU…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="pl-11 rounded-full"
        />
      </div>

      {isLoading ? (
        <div className="text-center py-16 font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">
          Curating products…
        </div>
      ) : products?.data?.length === 0 ? (
        <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line">
          <Package
            className="mx-auto h-10 w-10 text-ink-mute mb-4"
            strokeWidth={1.5}
          />
          <p className="font-display font-bold text-xl uppercase tracking-tight text-ink">No products yet</p>
          <p className="text-sm text-ink-soft mt-1">
            Begin by adding your first piece.
          </p>
        </div>
      ) : (
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>SKU</TableHead>
                <TableHead>Name</TableHead>
                <TableHead>Category</TableHead>
                <TableHead className="text-right">Price</TableHead>
                <TableHead className="text-right">Stock</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {products?.data?.map((product: any) => {
                const totalQty = product.stockLevels?.reduce((sum: number, sl: any) => sum + sl.qty, 0) || 0;
                const isLow = totalQty <= product.reorderLevel;
                return (
                  <TableRow key={product.id}>
                    <TableCell className="font-mono text-xs text-ink-soft tabular">
                      {product.sku}
                    </TableCell>
                    <TableCell className="font-medium text-ink">
                      {product.name}
                    </TableCell>
                    <TableCell className="text-ink-soft">
                      {product.category}
                    </TableCell>
                    <TableCell className="text-right tabular">
                      {formatCurrency(product.unitPrice)}
                    </TableCell>
                    <TableCell className="text-right tabular text-ink-soft">
                      {totalQty}
                    </TableCell>
                    <TableCell>
                      {isLow ? (
                        <Badge variant="warning">Low stock</Badge>
                      ) : (
                        <Badge variant="success">In stock</Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-0.5">
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => openEdit(product)}
                          className="h-8 w-8"
                        >
                          <Pencil className="h-3.5 w-3.5" strokeWidth={1.75} />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setDeleting(product)}
                          className="h-8 w-8 hover:text-danger"
                        >
                          <Trash2 className="h-3.5 w-3.5" strokeWidth={1.75} />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}
      <Pagination page={page} totalPages={products?.meta?.pages || 1} onPageChange={setPage} />

      {/* Add Dialog */}
      <Dialog open={showAdd} onClose={() => setShowAdd(false)}>
        <DialogHeader><DialogTitle>Add Product</DialogTitle></DialogHeader>
        <form onSubmit={handleCreate} className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>SKU</Label>
              <Input value={form.sku} onChange={(e) => setForm({ ...form, sku: e.target.value })} placeholder="PRD-001" required />
            </div>
            <div className="space-y-2">
              <Label>Category</Label>
              <Input value={form.category} onChange={(e) => setForm({ ...form, category: e.target.value })} />
            </div>
          </div>
          <div className="space-y-2">
            <Label>Product Name</Label>
            <Input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Selling Price</Label>
              <Input type="number" value={form.unitPrice} onChange={(e) => setForm({ ...form, unitPrice: e.target.value })} required />
            </div>
            <div className="space-y-2">
              <Label>Cost Price</Label>
              <Input type="number" value={form.costPrice} onChange={(e) => setForm({ ...form, costPrice: e.target.value })} />
            </div>
          </div>
          <div className="space-y-2">
            <Label>Reorder Level</Label>
            <Input type="number" value={form.reorderLevel} onChange={(e) => setForm({ ...form, reorderLevel: e.target.value })} />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setShowAdd(false)}>Cancel</Button>
            <Button type="submit" disabled={createProduct.isPending}>
              {createProduct.isPending ? "Creating..." : "Create Product"}
            </Button>
          </div>
        </form>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={!!editing} onClose={() => setEditing(null)}>
        <DialogHeader><DialogTitle>Edit Product</DialogTitle></DialogHeader>
        <form onSubmit={handleUpdate} className="space-y-4">
          <div className="space-y-2">
            <Label>Product Name</Label>
            <Input value={editing?.name || ""} onChange={(e) => setEditing({ ...editing, name: e.target.value })} required />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Selling Price</Label>
              <Input type="number" value={editing?.unitPrice || ""} onChange={(e) => setEditing({ ...editing, unitPrice: e.target.value })} required />
            </div>
            <div className="space-y-2">
              <Label>Cost Price</Label>
              <Input type="number" value={editing?.costPrice || ""} onChange={(e) => setEditing({ ...editing, costPrice: e.target.value })} />
            </div>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Category</Label>
              <Input value={editing?.category || ""} onChange={(e) => setEditing({ ...editing, category: e.target.value })} />
            </div>
            <div className="space-y-2">
              <Label>Reorder Level</Label>
              <Input type="number" value={editing?.reorderLevel || ""} onChange={(e) => setEditing({ ...editing, reorderLevel: e.target.value })} />
            </div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button type="submit" disabled={updateProduct.isPending}>
              {updateProduct.isPending ? "Saving..." : "Save Changes"}
            </Button>
          </div>
        </form>
      </Dialog>

      {/* Delete Confirm */}
      <ConfirmDialog
        open={!!deleting}
        onClose={() => setDeleting(null)}
        onConfirm={() => deleting && deleteProduct.mutate(deleting.id)}
        title="Delete Product"
        description={`Are you sure you want to delete "${deleting?.name}"? This action cannot be undone.`}
        confirmLabel="Delete"
        variant="destructive"
        loading={deleteProduct.isPending}
      />
    </div>
  );
}
