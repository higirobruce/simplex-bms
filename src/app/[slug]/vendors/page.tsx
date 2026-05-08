"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { useTenant, useDebounce } from "@/lib/hooks";
import { Pagination } from "@/components/ui/pagination";
import { PageHeader } from "@/components/page-header";
import { Plus, Search, Truck, Pencil, Trash2 } from "lucide-react";

function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, { ...opts, headers: { "Content-Type": "application/json", ...opts?.headers } }).then(async (r) => {
    if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Request failed"); }
    return r.json();
  });
}

export default function VendorsPage() {
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const debouncedSearch = useDebounce(search, 300);
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<any>(null);
  const [deleting, setDeleting] = useState<any>(null);
  const emptyForm = { name: "", email: "", phone: "", address: "", city: "", country: "", paymentTerms: "NET30" };
  const [form, setForm] = useState(emptyForm);

  useEffect(() => { setPage(1); }, [debouncedSearch]);

  const { data: vendors, isLoading } = useQuery({
    queryKey: ["vendors", slug, page, debouncedSearch],
    queryFn: () => fetch(`/api/${slug}/vendors?page=${page}&limit=20&search=${debouncedSearch}`).then((r) => r.json()),
    enabled: !!slug,
  });

  const create = useMutation({
    mutationFn: (data: any) => apiCall(`/api/${slug}/vendors`, { method: "POST", body: JSON.stringify(data) }),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["vendors", slug] }); setShowAdd(false); setForm(emptyForm); toast.success("Vendor created"); },
    onError: (err: Error) => toast.error(err.message),
  });

  const update = useMutation({
    mutationFn: ({ id, data }: { id: string; data: any }) => apiCall(`/api/${slug}/vendors/${id}`, { method: "PATCH", body: JSON.stringify(data) }),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["vendors", slug] }); setEditing(null); toast.success("Vendor updated"); },
    onError: (err: Error) => toast.error(err.message),
  });

  const remove = useMutation({
    mutationFn: (id: string) => apiCall(`/api/${slug}/vendors/${id}`, { method: "DELETE" }),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["vendors", slug] }); setDeleting(null); toast.success("Vendor deleted"); },
    onError: (err: Error) => toast.error(err.message),
  });

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Relationships"
        title="Vendors"
        description="The houses you source from — payment terms, contacts, and standing."
        actions={
          <Button onClick={() => setShowAdd(true)} size="lg">
            <Plus className="h-4 w-4" strokeWidth={2} /> Add Vendor
          </Button>
        }
      />
      <div className="relative mb-6 max-w-md">
        <Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute" strokeWidth={1.75} />
        <Input placeholder="Search by name or email…" value={search} onChange={(e) => setSearch(e.target.value)} className="pl-11 rounded-full" />
      </div>
      {isLoading ? <div className="text-center py-16 text-ink-mute font-serif italic">Loading…</div> : vendors?.data?.length === 0 ? (
        <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line"><Truck className="mx-auto h-10 w-10 text-ink-mute mb-4" strokeWidth={1.5} /><p className="font-serif italic text-lg text-ink">No vendors yet</p></div>
      ) : (
        <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden">
          <Table>
            <TableHeader><TableRow><TableHead>Name</TableHead><TableHead>Email</TableHead><TableHead>Phone</TableHead><TableHead>Terms</TableHead><TableHead>Status</TableHead><TableHead className="text-right">Actions</TableHead></TableRow></TableHeader>
            <TableBody>
              {vendors?.data?.map((v: any) => (
                <TableRow key={v.id}>
                  <TableCell className="font-medium text-ink">{v.name}</TableCell>
                  <TableCell className="text-ink-soft">{v.email || "—"}</TableCell>
                  <TableCell className="text-ink-soft">{v.phone || "—"}</TableCell>
                  <TableCell><Badge variant="outline">{v.paymentTerms}</Badge></TableCell>
                  <TableCell><Badge variant={v.status === "ACTIVE" ? "success" : "secondary"}>{v.status?.toLowerCase()}</Badge></TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-0.5">
                      <Button variant="ghost" size="icon" onClick={() => setEditing({ ...v })} className="h-8 w-8"><Pencil className="h-3.5 w-3.5" strokeWidth={1.75} /></Button>
                      <Button variant="ghost" size="icon" onClick={() => setDeleting(v)} className="h-8 w-8 hover:text-danger"><Trash2 className="h-3.5 w-3.5" strokeWidth={1.75} /></Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <Pagination page={page} totalPages={vendors?.meta?.pages || 1} onPageChange={setPage} />
      <Dialog open={showAdd} onClose={() => setShowAdd(false)}>
        <DialogHeader><DialogTitle>Add Vendor</DialogTitle></DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); create.mutate(form); }} className="space-y-4">
          <div className="space-y-2"><Label>Company Name</Label><Input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required /></div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>Email</Label><Input type="email" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} /></div>
            <div className="space-y-2"><Label>Phone</Label><Input value={form.phone} onChange={(e) => setForm({ ...form, phone: e.target.value })} /></div>
          </div>
          <div className="space-y-2"><Label>Payment Terms</Label>
            <Select value={form.paymentTerms} onChange={(e) => setForm({ ...form, paymentTerms: e.target.value })}>
              <option value="COD">Cash on Delivery</option><option value="NET15">Net 15</option><option value="NET30">Net 30</option><option value="NET60">Net 60</option>
            </Select>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>City</Label><Input value={form.city} onChange={(e) => setForm({ ...form, city: e.target.value })} /></div>
            <div className="space-y-2"><Label>Country</Label><Input value={form.country} onChange={(e) => setForm({ ...form, country: e.target.value })} /></div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setShowAdd(false)}>Cancel</Button>
            <Button type="submit" disabled={create.isPending}>{create.isPending ? "Creating..." : "Add Vendor"}</Button>
          </div>
        </form>
      </Dialog>
      <Dialog open={!!editing} onClose={() => setEditing(null)}>
        <DialogHeader><DialogTitle>Edit Vendor</DialogTitle></DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); update.mutate({ id: editing.id, data: { name: editing.name, email: editing.email || null, phone: editing.phone || null, paymentTerms: editing.paymentTerms, city: editing.city || null, country: editing.country || null } }); }} className="space-y-4">
          <div className="space-y-2"><Label>Company Name</Label><Input value={editing?.name || ""} onChange={(e) => setEditing({ ...editing, name: e.target.value })} required /></div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>Email</Label><Input type="email" value={editing?.email || ""} onChange={(e) => setEditing({ ...editing, email: e.target.value })} /></div>
            <div className="space-y-2"><Label>Phone</Label><Input value={editing?.phone || ""} onChange={(e) => setEditing({ ...editing, phone: e.target.value })} /></div>
          </div>
          <div className="space-y-2"><Label>Payment Terms</Label>
            <Select value={editing?.paymentTerms || "NET30"} onChange={(e) => setEditing({ ...editing, paymentTerms: e.target.value })}>
              <option value="COD">Cash on Delivery</option><option value="NET15">Net 15</option><option value="NET30">Net 30</option><option value="NET60">Net 60</option>
            </Select>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button type="submit" disabled={update.isPending}>{update.isPending ? "Saving..." : "Save Changes"}</Button>
          </div>
        </form>
      </Dialog>
      <ConfirmDialog open={!!deleting} onClose={() => setDeleting(null)} onConfirm={() => deleting && remove.mutate(deleting.id)} title="Delete Vendor" description={`Delete "${deleting?.name}"? This cannot be undone.`} confirmLabel="Delete" variant="destructive" loading={remove.isPending} />
    </div>
  );
}
