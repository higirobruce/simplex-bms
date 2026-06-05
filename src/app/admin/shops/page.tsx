"use client";

import { useState, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/ui/pagination";
import { apiCall } from "@/lib/fetcher";
import { useDebounce, usePlatform } from "@/lib/hooks";
import { Plus, Pencil, Trash2, LogIn, Ban, CheckCircle2, Search } from "lucide-react";

function slugify(s: string) {
  return s
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

const emptyCreate = {
  name: "",
  slug: "",
  currency: "RWF",
  timezone: "Africa/Kigali",
  adminEmail: "",
  adminName: "",
  adminPassword: "",
};

export default function ShopsPage() {
  const queryClient = useQueryClient();
  const toast = useToast();
  const platform = usePlatform();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const debouncedSearch = useDebounce(search, 300);
  useEffect(() => { setPage(1); }, [debouncedSearch]);

  const [showAdd, setShowAdd] = useState(false);
  const [createForm, setCreateForm] = useState(emptyCreate);
  const [slugEdited, setSlugEdited] = useState(false);
  const [editing, setEditing] = useState<any>(null);
  const [editForm, setEditForm] = useState({ name: "", currency: "", timezone: "", logo: "" });
  const [deleting, setDeleting] = useState<any>(null);
  const [entering, setEntering] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["admin-shops", page, debouncedSearch],
    queryFn: () =>
      apiCall(`/api/admin/orgs?page=${page}&limit=25&search=${encodeURIComponent(debouncedSearch)}`),
  });

  const create = useMutation({
    mutationFn: (body: any) => apiCall("/api/admin/orgs", { method: "POST", body: JSON.stringify(body) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["admin-shops"] });
      setShowAdd(false);
      setCreateForm(emptyCreate);
      setSlugEdited(false);
      toast.success("Shop created");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const update = useMutation({
    mutationFn: ({ id, body }: { id: string; body: any }) =>
      apiCall(`/api/admin/orgs/${id}`, { method: "PATCH", body: JSON.stringify(body) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["admin-shops"] });
      setEditing(null);
      toast.success("Shop updated");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const remove = useMutation({
    mutationFn: (id: string) => apiCall(`/api/admin/orgs/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["admin-shops"] });
      setDeleting(null);
      toast.success("Shop deleted");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  async function enterShop(shop: any) {
    setEntering(shop.id);
    try {
      const res = await apiCall("/api/admin/impersonate", {
        method: "POST",
        body: JSON.stringify({ orgId: shop.id }),
      });
      // Hard navigation so the freshly-set acting-org cookie is sent and
      // middleware re-evaluates server-side (client router.push can race it).
      window.location.assign(`/${res.slug}/dashboard`);
    } catch (e: any) {
      toast.error(e.message);
      setEntering(null);
    }
  }

  function openCreate() {
    setCreateForm({
      ...emptyCreate,
      currency: platform?.defaultCurrency ?? emptyCreate.currency,
      timezone: platform?.defaultTimezone ?? emptyCreate.timezone,
    });
    setSlugEdited(false);
    setShowAdd(true);
  }

  function openEdit(shop: any) {
    setEditForm({
      name: shop.name ?? "",
      currency: shop.currency ?? "RWF",
      timezone: shop.timezone ?? "Africa/Kigali",
      logo: shop.logo ?? "",
    });
    setEditing(shop);
  }

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Platform"
        title="Shops"
        description={`Every workspace on ${platform?.platformName ?? "Simplex"}. Provision new shops, suspend access, or step inside.`}
        actions={
          <Button onClick={openCreate}>
            <Plus className="h-4 w-4" strokeWidth={2} /> New shop
          </Button>
        }
      />

      <div className="relative mb-4 max-w-sm">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute" strokeWidth={1.75} />
        <Input
          className="pl-9"
          placeholder="Search shops…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      <Card>
        <CardContent className="py-2">
          {isLoading ? (
            <div className="py-10 text-center font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">
              Loading…
            </div>
          ) : (data?.data?.length ?? 0) === 0 ? (
            <div className="py-10 text-center text-sm text-ink-mute">No shops found.</div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Shop</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Users</TableHead>
                  <TableHead>Currency</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.data.map((shop: any) => {
                  const suspended = shop.status === "SUSPENDED";
                  return (
                    <TableRow key={shop.id}>
                      <TableCell>
                        <div className="font-medium text-ink">{shop.name}</div>
                        <div className="font-mono text-[0.65rem] tracking-[0.1em] uppercase text-ink-mute mt-0.5">
                          /{shop.slug}
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge variant={suspended ? "destructive" : "success"}>
                          {shop.status.toLowerCase()}
                        </Badge>
                      </TableCell>
                      <TableCell className="text-ink-soft">{shop._count.users}</TableCell>
                      <TableCell className="text-ink-soft">{shop.currency}</TableCell>
                      <TableCell className="text-right">
                        <div className="flex justify-end gap-0.5">
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => enterShop(shop)}
                            disabled={suspended || entering === shop.id}
                            title={suspended ? "Suspended shops cannot be entered" : "Enter shop"}
                          >
                            <LogIn className="h-3.5 w-3.5" strokeWidth={1.75} />
                            {entering === shop.id ? "Entering…" : "Enter"}
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8"
                            title={suspended ? "Activate" : "Suspend"}
                            onClick={() =>
                              update.mutate({
                                id: shop.id,
                                body: { status: suspended ? "ACTIVE" : "SUSPENDED" },
                              })
                            }
                          >
                            {suspended ? (
                              <CheckCircle2 className="h-3.5 w-3.5 text-success" strokeWidth={1.75} />
                            ) : (
                              <Ban className="h-3.5 w-3.5" strokeWidth={1.75} />
                            )}
                          </Button>
                          <Button variant="ghost" size="icon" className="h-8 w-8" title="Edit" onClick={() => openEdit(shop)}>
                            <Pencil className="h-3.5 w-3.5" strokeWidth={1.75} />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 hover:text-danger"
                            title="Delete"
                            onClick={() => setDeleting(shop)}
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
          )}
        </CardContent>
      </Card>

      <Pagination page={page} totalPages={data?.meta?.pages || 1} onPageChange={setPage} />

      {/* Create shop */}
      <Dialog open={showAdd} onClose={() => setShowAdd(false)}>
        <DialogHeader>
          <DialogTitle>New shop</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate({
              name: createForm.name,
              slug: createForm.slug,
              currency: createForm.currency,
              timezone: createForm.timezone,
              adminEmail: createForm.adminEmail,
              adminName: createForm.adminName || null,
              adminPassword: createForm.adminPassword,
            });
          }}
          className="space-y-4"
        >
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Shop name</Label>
              <Input
                value={createForm.name}
                onChange={(e) => {
                  const name = e.target.value;
                  setCreateForm((f) => ({
                    ...f,
                    name,
                    slug: slugEdited ? f.slug : slugify(name),
                  }));
                }}
                required
              />
            </div>
            <div className="space-y-2">
              <Label>Slug</Label>
              <Input
                value={createForm.slug}
                onChange={(e) => {
                  setSlugEdited(true);
                  setCreateForm({ ...createForm, slug: slugify(e.target.value) });
                }}
                required
              />
            </div>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Currency</Label>
              <Input
                value={createForm.currency}
                onChange={(e) => setCreateForm({ ...createForm, currency: e.target.value.toUpperCase() })}
                maxLength={8}
                required
              />
            </div>
            <div className="space-y-2">
              <Label>Timezone</Label>
              <Input
                value={createForm.timezone}
                onChange={(e) => setCreateForm({ ...createForm, timezone: e.target.value })}
                required
              />
            </div>
          </div>

          <div className="pt-2 border-t border-line/70">
            <p className="font-mono text-[0.62rem] tracking-[0.18em] uppercase text-ink-mute mb-3 mt-3">
              First admin
            </p>
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <Label>Admin email</Label>
                  <Input
                    type="email"
                    value={createForm.adminEmail}
                    onChange={(e) => setCreateForm({ ...createForm, adminEmail: e.target.value })}
                    required
                  />
                </div>
                <div className="space-y-2">
                  <Label>Admin name</Label>
                  <Input
                    value={createForm.adminName}
                    onChange={(e) => setCreateForm({ ...createForm, adminName: e.target.value })}
                  />
                </div>
              </div>
              <div className="space-y-2">
                <Label>Temporary password</Label>
                <Input
                  type="password"
                  value={createForm.adminPassword}
                  onChange={(e) => setCreateForm({ ...createForm, adminPassword: e.target.value })}
                  required
                  minLength={8}
                />
              </div>
            </div>
          </div>

          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setShowAdd(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending}>
              {create.isPending ? "Creating…" : "Create shop"}
            </Button>
          </div>
        </form>
      </Dialog>

      {/* Edit shop */}
      <Dialog open={!!editing} onClose={() => setEditing(null)}>
        <DialogHeader>
          <DialogTitle>Edit shop</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (!editing) return;
            update.mutate({
              id: editing.id,
              body: {
                name: editForm.name,
                currency: editForm.currency,
                timezone: editForm.timezone,
                logo: editForm.logo.trim() || null,
              },
            });
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Slug</Label>
            <Input value={editing?.slug ?? ""} disabled />
          </div>
          <div className="space-y-2">
            <Label>Name</Label>
            <Input value={editForm.name} onChange={(e) => setEditForm({ ...editForm, name: e.target.value })} required />
          </div>
          <div className="space-y-2">
            <Label>Logo URL</Label>
            <Input
              type="url"
              placeholder="https://…"
              value={editForm.logo}
              onChange={(e) => setEditForm({ ...editForm, logo: e.target.value })}
            />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Currency</Label>
              <Input
                value={editForm.currency}
                onChange={(e) => setEditForm({ ...editForm, currency: e.target.value.toUpperCase() })}
                maxLength={8}
                required
              />
            </div>
            <div className="space-y-2">
              <Label>Timezone</Label>
              <Input
                value={editForm.timezone}
                onChange={(e) => setEditForm({ ...editForm, timezone: e.target.value })}
                required
              />
            </div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button type="submit" disabled={update.isPending}>
              {update.isPending ? "Saving…" : "Save changes"}
            </Button>
          </div>
        </form>
      </Dialog>

      <ConfirmDialog
        open={!!deleting}
        onClose={() => setDeleting(null)}
        onConfirm={() => deleting && remove.mutate(deleting.id)}
        title="Delete shop"
        description={`Permanently delete "${deleting?.name}" and ALL of its data (users, products, invoices, stock…)? This cannot be undone.`}
        confirmLabel="Delete shop"
        variant="destructive"
        loading={remove.isPending}
      />
    </div>
  );
}
