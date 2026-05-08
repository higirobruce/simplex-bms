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
import { formatCurrency, formatDate } from "@/lib/utils";
import { Plus, Search, DollarSign, Pencil, Trash2 } from "lucide-react";

const categories = ["rent", "utilities", "salary", "supplies", "transport", "marketing", "other"];

function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, { ...opts, headers: { "Content-Type": "application/json", ...opts?.headers } }).then(async (r) => {
    if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Request failed"); }
    return r.json();
  });
}

export default function ExpensesPage() {
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const debouncedSearch = useDebounce(search, 300);
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<any>(null);
  const [deleting, setDeleting] = useState<any>(null);
  const emptyForm = { category: "other", amount: "", date: new Date().toISOString().split("T")[0], description: "" };
  const [form, setForm] = useState(emptyForm);

  useEffect(() => { setPage(1); }, [debouncedSearch]);

  const { data: expenses, isLoading } = useQuery({
    queryKey: ["expenses", slug, page, debouncedSearch],
    queryFn: () => fetch(`/api/${slug}/expenses?page=${page}&limit=20&search=${debouncedSearch}`).then((r) => r.json()),
    enabled: !!slug,
  });

  const create = useMutation({
    mutationFn: (data: any) => apiCall(`/api/${slug}/expenses`, { method: "POST", body: JSON.stringify(data) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["expenses", slug] });
      queryClient.invalidateQueries({ queryKey: ["dashboard-stats", slug] });
      setShowAdd(false); setForm(emptyForm); toast.success("Expense added");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const update = useMutation({
    mutationFn: ({ id, data }: { id: string; data: any }) => apiCall(`/api/${slug}/expenses/${id}`, { method: "PATCH", body: JSON.stringify(data) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["expenses", slug] });
      queryClient.invalidateQueries({ queryKey: ["dashboard-stats", slug] });
      setEditing(null); toast.success("Expense updated");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const remove = useMutation({
    mutationFn: (id: string) => apiCall(`/api/${slug}/expenses/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["expenses", slug] });
      queryClient.invalidateQueries({ queryKey: ["dashboard-stats", slug] });
      setDeleting(null); toast.success("Expense deleted");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const totalExpenses = expenses?.data?.reduce((sum: number, e: any) => sum + Number(e.amount), 0) || 0;

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Commerce"
        title="Expenses"
        description="The outlays of running your house — categorised and recorded with care."
        actions={
          <Button onClick={() => setShowAdd(true)} size="lg">
            <Plus className="h-4 w-4" strokeWidth={2} /> Add Expense
          </Button>
        }
      />
      <div className="relative mb-6 max-w-md">
        <Search className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute" strokeWidth={1.75} />
        <Input placeholder="Search by category or description…" value={search} onChange={(e) => setSearch(e.target.value)} className="pl-11 rounded-full" />
      </div>
      {isLoading ? <div className="text-center py-16 font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">Loading…</div> : expenses?.data?.length === 0 ? (
        <div className="text-center py-20 rounded-[var(--radius-lg)] border border-dashed border-line"><DollarSign className="mx-auto h-10 w-10 text-ink-mute mb-4" strokeWidth={1.5} /><p className="font-display font-bold text-xl uppercase tracking-tight text-ink">No expenses yet</p></div>
      ) : (
        <>
          <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden mb-4">
            <Table>
              <TableHeader><TableRow><TableHead>Date</TableHead><TableHead>Category</TableHead><TableHead>Description</TableHead><TableHead className="text-right">Amount</TableHead><TableHead className="text-right">Actions</TableHead></TableRow></TableHeader>
              <TableBody>
                {expenses?.data?.map((expense: any) => (
                  <TableRow key={expense.id}>
                    <TableCell className="text-ink-soft tabular">{formatDate(expense.date)}</TableCell>
                    <TableCell><Badge variant="outline" className="capitalize">{expense.category}</Badge></TableCell>
                    <TableCell className="text-ink">{expense.description || "—"}</TableCell>
                    <TableCell className="text-right font-medium tabular">{formatCurrency(expense.amount)}</TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-0.5">
                        <Button variant="ghost" size="icon" onClick={() => setEditing({ ...expense, amount: String(expense.amount), date: new Date(expense.date).toISOString().split("T")[0] })} className="h-8 w-8"><Pencil className="h-3.5 w-3.5" strokeWidth={1.75} /></Button>
                        <Button variant="ghost" size="icon" onClick={() => setDeleting(expense)} className="h-8 w-8 hover:text-danger"><Trash2 className="h-3.5 w-3.5" strokeWidth={1.75} /></Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
          <div className="text-right text-sm text-ink-soft">Total: <span className="font-display text-xl font-semibold uppercase tracking-tight text-ink ml-1 tabular">{formatCurrency(totalExpenses)}</span></div>
          <Pagination page={page} totalPages={expenses?.meta?.pages || 1} onPageChange={setPage} />
        </>
      )}
      <Dialog open={showAdd} onClose={() => setShowAdd(false)}>
        <DialogHeader><DialogTitle>Add Expense</DialogTitle></DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); create.mutate({ ...form, amount: parseFloat(form.amount) }); }} className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>Category</Label>
              <Select value={form.category} onChange={(e) => setForm({ ...form, category: e.target.value })}>
                {categories.map((c) => <option key={c} value={c}>{c.charAt(0).toUpperCase() + c.slice(1)}</option>)}
              </Select>
            </div>
            <div className="space-y-2"><Label>Amount</Label><Input type="number" min="0" value={form.amount} onChange={(e) => setForm({ ...form, amount: e.target.value })} required /></div>
          </div>
          <div className="space-y-2"><Label>Date</Label><Input type="date" value={form.date} onChange={(e) => setForm({ ...form, date: e.target.value })} required /></div>
          <div className="space-y-2"><Label>Description</Label><Input value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} placeholder="What was this for?" /></div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setShowAdd(false)}>Cancel</Button>
            <Button type="submit" disabled={create.isPending}>{create.isPending ? "Adding..." : "Add Expense"}</Button>
          </div>
        </form>
      </Dialog>
      <Dialog open={!!editing} onClose={() => setEditing(null)}>
        <DialogHeader><DialogTitle>Edit Expense</DialogTitle></DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); update.mutate({ id: editing.id, data: { category: editing.category, amount: parseFloat(editing.amount), date: editing.date, description: editing.description || null } }); }} className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2"><Label>Category</Label>
              <Select value={editing?.category || "other"} onChange={(e) => setEditing({ ...editing, category: e.target.value })}>
                {categories.map((c) => <option key={c} value={c}>{c.charAt(0).toUpperCase() + c.slice(1)}</option>)}
              </Select>
            </div>
            <div className="space-y-2"><Label>Amount</Label><Input type="number" value={editing?.amount || ""} onChange={(e) => setEditing({ ...editing, amount: e.target.value })} required /></div>
          </div>
          <div className="space-y-2"><Label>Date</Label><Input type="date" value={editing?.date || ""} onChange={(e) => setEditing({ ...editing, date: e.target.value })} required /></div>
          <div className="space-y-2"><Label>Description</Label><Input value={editing?.description || ""} onChange={(e) => setEditing({ ...editing, description: e.target.value })} /></div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button type="submit" disabled={update.isPending}>{update.isPending ? "Saving..." : "Save Changes"}</Button>
          </div>
        </form>
      </Dialog>
      <ConfirmDialog open={!!deleting} onClose={() => setDeleting(null)} onConfirm={() => deleting && remove.mutate(deleting.id)} title="Delete Expense" description="Are you sure? This cannot be undone." confirmLabel="Delete" variant="destructive" loading={remove.isPending} />
    </div>
  );
}
