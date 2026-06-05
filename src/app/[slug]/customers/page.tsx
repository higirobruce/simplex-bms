"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { useTenant, useDebounce } from "@/lib/hooks";
import { Pagination } from "@/components/ui/pagination";
import { PageHeader } from "@/components/page-header";
import { Plus, Search, Users, Pencil, Trash2 } from "lucide-react";

function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, { ...opts, headers: { "Content-Type": "application/json", ...opts?.headers } }).then(async (r) => {
    if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Request failed"); }
    return r.json();
  });
}

export default function CustomersPage() {
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const debouncedSearch = useDebounce(search, 300);
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<any>(null);
  const [deleting, setDeleting] = useState<any>(null);
  const emptyForm = { name: "", email: "", phone: "", address: "", city: "", country: "" };
  const [form, setForm] = useState(emptyForm);

  useEffect(() => { setPage(1); }, [debouncedSearch]);

  const { data: customers, isLoading } = useQuery({
    queryKey: ["customers", slug, page, debouncedSearch],
    queryFn: () => fetch(`/api/${slug}/customers?page=${page}&limit=20&search=${debouncedSearch}`).then((r) => r.json()),
    enabled: !!slug,
  });

  const create = useMutation({
    mutationFn: (data: any) => apiCall(`/api/${slug}/customers`, { method: "POST", body: JSON.stringify(data) }),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["customers", slug] }); setShowAdd(false); setForm(emptyForm); toast.success("Customer created"); },
    onError: (err: Error) => toast.error(err.message),
  });

  const update = useMutation({
    mutationFn: ({ id, data }: { id: string; data: any }) => apiCall(`/api/${slug}/customers/${id}`, { method: "PATCH", body: JSON.stringify(data) }),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["customers", slug] }); setEditing(null); toast.success("Customer updated"); },
    onError: (err: Error) => toast.error(err.message),
  });

  const remove = useMutation({
    mutationFn: (id: string) => apiCall(`/api/${slug}/customers/${id}`, { method: "DELETE" }),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["customers", slug] }); setDeleting(null); toast.success("Customer deleted"); },
    onError: (err: Error) => toast.error(err.message),
  });

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Relationships"
        title="Customers"
        description="The patrons of your house — their details, history, and standing."
        actions={
          <Button onClick={() => setShowAdd(true)} size="lg">
            <Plus className="h-4 w-4" strokeWidth={2} /> Add Customer
          </Button>
        }
      />
      <div className="relative mb-6 max-w-md">
        <Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute" strokeWidth={1.75} />
        <Input placeholder="Search by name, email, or phone…" value={search} onChange={(e) => setSearch(e.target.value)} className="pl-11 rounded-full" />
      </div>
      {isLoading ? <div className="text-center py-16 font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">Loading…</div> : customers?.data?.length === 0 ? (
        <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line"><Users className="mx-auto h-10 w-10 text-ink-mute mb-4" strokeWidth={1.5} /><p className="font-display font-bold text-xl uppercase tracking-tight text-ink">No customers yet</p></div>
      ) : (
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead><TableHead>Email</TableHead><TableHead>Phone</TableHead><TableHead>City</TableHead>
                <TableHead className="text-right">Invoices</TableHead><TableHead>Status</TableHead><TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {customers?.data?.map((c: any) => (
                <TableRow key={c.id}>
                  <TableCell className="font-medium text-ink">{c.name}</TableCell>
                  <TableCell className="text-ink-soft">{c.email || "—"}</TableCell>
                  <TableCell className="text-ink-soft">{c.phone || "—"}</TableCell>
                  <TableCell className="text-ink-soft">{c.city || "—"}</TableCell>
                  <TableCell className="text-right tabular text-ink-soft">{c._count?.invoices || 0}</TableCell>
                  <TableCell><Badge variant={c.status === "ACTIVE" ? "success" : "secondary"}>{c.status?.toLowerCase()}</Badge></TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-0.5">
                      <Button variant="ghost" size="icon" onClick={() => setEditing({ ...c })} className="h-8 w-8"><Pencil className="h-3.5 w-3.5" strokeWidth={1.75} /></Button>
                      <Button variant="ghost" size="icon" onClick={() => setDeleting(c)} className="h-8 w-8 hover:text-danger"><Trash2 className="h-3.5 w-3.5" strokeWidth={1.75} /></Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <Pagination page={page} totalPages={customers?.meta?.pages || 1} onPageChange={setPage} />
      {/* Add */}
      <Dialog open={showAdd} onClose={() => setShowAdd(false)}>
        <DialogHeader><DialogTitle>Add Customer</DialogTitle></DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); create.mutate(form); }} className="space-y-4">
          <div className="space-y-2"><Label>Name</Label><Input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required /></div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>Email</Label><Input type="email" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} /></div>
            <div className="space-y-2"><Label>Phone</Label><Input value={form.phone} onChange={(e) => setForm({ ...form, phone: e.target.value })} /></div>
          </div>
          <div className="space-y-2"><Label>Address</Label><Input value={form.address} onChange={(e) => setForm({ ...form, address: e.target.value })} /></div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>City</Label><Input value={form.city} onChange={(e) => setForm({ ...form, city: e.target.value })} /></div>
            <div className="space-y-2"><Label>Country</Label><Input value={form.country} onChange={(e) => setForm({ ...form, country: e.target.value })} /></div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setShowAdd(false)}>Cancel</Button>
            <Button type="submit" disabled={create.isPending}>{create.isPending ? "Creating..." : "Add Customer"}</Button>
          </div>
        </form>
      </Dialog>
      {/* Edit */}
      <Dialog open={!!editing} onClose={() => setEditing(null)}>
        <DialogHeader><DialogTitle>Edit Customer</DialogTitle></DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); update.mutate({ id: editing.id, data: { name: editing.name, email: editing.email || null, phone: editing.phone || null, address: editing.address || null, city: editing.city || null, country: editing.country || null } }); }} className="space-y-4">
          <div className="space-y-2"><Label>Name</Label><Input value={editing?.name || ""} onChange={(e) => setEditing({ ...editing, name: e.target.value })} required /></div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>Email</Label><Input type="email" value={editing?.email || ""} onChange={(e) => setEditing({ ...editing, email: e.target.value })} /></div>
            <div className="space-y-2"><Label>Phone</Label><Input value={editing?.phone || ""} onChange={(e) => setEditing({ ...editing, phone: e.target.value })} /></div>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>City</Label><Input value={editing?.city || ""} onChange={(e) => setEditing({ ...editing, city: e.target.value })} /></div>
            <div className="space-y-2"><Label>Country</Label><Input value={editing?.country || ""} onChange={(e) => setEditing({ ...editing, country: e.target.value })} /></div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button type="submit" disabled={update.isPending}>{update.isPending ? "Saving..." : "Save Changes"}</Button>
          </div>
        </form>
      </Dialog>
      <ConfirmDialog open={!!deleting} onClose={() => setDeleting(null)} onConfirm={() => deleting && remove.mutate(deleting.id)} title="Delete Customer" description={`Delete "${deleting?.name}"? This cannot be undone.`} confirmLabel="Delete" variant="destructive" loading={remove.isPending} />
    </div>
  );
}
