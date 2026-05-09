"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useTenant } from "@/lib/hooks";
import { PageHeader } from "@/components/page-header";
import { Building2, User, Shield, Pencil, Plus, Trash2, KeyRound, Users as UsersIcon } from "lucide-react";

const ROLES = ["ADMIN", "MANAGER", "ACCOUNTANT", "VIEWER"] as const;
type Role = (typeof ROLES)[number];

function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, { ...opts, headers: { "Content-Type": "application/json", ...opts?.headers } }).then(async (r) => {
    if (!r.ok) {
      const d = await r.json().catch(() => ({}));
      throw new Error(d.error || "Request failed");
    }
    return r.json();
  });
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between py-3 border-b border-line/70 last:border-0">
      <dt className="text-[0.7rem] tracking-[0.18em] uppercase text-ink-mute font-medium">{label}</dt>
      <dd className="text-sm text-ink">{value}</dd>
    </div>
  );
}

export default function SettingsPage() {
  const { slug, role, user } = useTenant();
  const isAdmin = role === "ADMIN";

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Workspace"
        title="Settings"
        description="The particulars of your workshop — organisation, people, and access."
      />

      <div className="grid gap-5 max-w-3xl">
        <OrganisationCard slug={slug} canEdit={isAdmin} />
        {isAdmin && <UsersCard slug={slug} currentUserId={(user as any)?.id} />}
        <AccountCard email={user?.email} name={user?.name} />
        <RoleCard role={role} />
      </div>
    </div>
  );
}

function OrganisationCard({ slug, canEdit }: { slug: string; canEdit: boolean }) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState<{ name: string; logo: string; currency: string; timezone: string }>({
    name: "",
    logo: "",
    currency: "",
    timezone: "",
  });

  const { data: org, isLoading } = useQuery({
    queryKey: ["org", slug],
    queryFn: () => apiCall(`/api/${slug}/org`),
    enabled: !!slug,
  });

  const update = useMutation({
    mutationFn: (data: any) => apiCall(`/api/${slug}/org`, { method: "PATCH", body: JSON.stringify(data) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["org", slug] });
      setEditing(false);
      toast.success("Organisation updated");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  function openEdit() {
    setForm({
      name: org?.name ?? "",
      logo: org?.logo ?? "",
      currency: org?.currency ?? "RWF",
      timezone: org?.timezone ?? "Africa/Kigali",
    });
    setEditing(true);
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="flex items-center gap-3 text-xl">
            <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
              <Building2 className="h-4 w-4 text-accent" strokeWidth={1.75} />
            </span>
            Organisation
          </CardTitle>
          {canEdit && (
            <Button variant="outline" size="sm" onClick={openEdit} disabled={isLoading || !org}>
              <Pencil className="h-3.5 w-3.5" strokeWidth={1.75} /> Edit
            </Button>
          )}
        </div>
      </CardHeader>
      <CardContent>
        <dl>
          <Row label="Name" value={isLoading ? "—" : org?.name ?? "—"} />
          <Row label="Slug" value={slug} />
          <Row label="Logo" value={org?.logo ? <a href={org.logo} target="_blank" rel="noreferrer" className="text-accent underline-offset-2 hover:underline">{org.logo}</a> : "—"} />
          <Row label="Currency" value={org?.currency ?? "—"} />
          <Row label="Timezone" value={org?.timezone ?? "—"} />
        </dl>
      </CardContent>

      <Dialog open={editing} onClose={() => setEditing(false)}>
        <DialogHeader>
          <DialogTitle>Edit Organisation</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            update.mutate({
              name: form.name,
              logo: form.logo.trim() || null,
              currency: form.currency,
              timezone: form.timezone,
            });
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Name</Label>
            <Input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />
          </div>
          <div className="space-y-2">
            <Label>Logo URL</Label>
            <Input
              type="url"
              placeholder="https://…"
              value={form.logo}
              onChange={(e) => setForm({ ...form, logo: e.target.value })}
            />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Currency</Label>
              <Input value={form.currency} onChange={(e) => setForm({ ...form, currency: e.target.value.toUpperCase() })} required maxLength={8} />
            </div>
            <div className="space-y-2">
              <Label>Timezone</Label>
              <Input value={form.timezone} onChange={(e) => setForm({ ...form, timezone: e.target.value })} required />
            </div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setEditing(false)}>Cancel</Button>
            <Button type="submit" disabled={update.isPending}>{update.isPending ? "Saving..." : "Save changes"}</Button>
          </div>
        </form>
      </Dialog>
    </Card>
  );
}

function UsersCard({ slug, currentUserId }: { slug: string; currentUserId?: string }) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<any>(null);
  const [resetting, setResetting] = useState<any>(null);
  const [deleting, setDeleting] = useState<any>(null);
  const emptyAdd = { email: "", name: "", password: "", role: "VIEWER" as Role };
  const [addForm, setAddForm] = useState(emptyAdd);
  const [editForm, setEditForm] = useState<{ name: string; role: Role }>({ name: "", role: "VIEWER" });
  const [newPassword, setNewPassword] = useState("");

  const { data: users, isLoading } = useQuery({
    queryKey: ["users", slug],
    queryFn: () => apiCall(`/api/${slug}/users?limit=100`),
    enabled: !!slug,
  });

  const create = useMutation({
    mutationFn: (data: any) => apiCall(`/api/${slug}/users`, { method: "POST", body: JSON.stringify(data) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["users", slug] });
      setShowAdd(false);
      setAddForm(emptyAdd);
      toast.success("User created");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const update = useMutation({
    mutationFn: ({ id, data }: { id: string; data: any }) =>
      apiCall(`/api/${slug}/users/${id}`, { method: "PATCH", body: JSON.stringify(data) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["users", slug] });
      setEditing(null);
      setResetting(null);
      setNewPassword("");
      toast.success("User updated");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const remove = useMutation({
    mutationFn: (id: string) => apiCall(`/api/${slug}/users/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["users", slug] });
      setDeleting(null);
      toast.success("User deleted");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  function openEdit(u: any) {
    setEditForm({ name: u.name ?? "", role: u.role });
    setEditing(u);
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="flex items-center gap-3 text-xl">
            <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
              <UsersIcon className="h-4 w-4 text-accent" strokeWidth={1.75} />
            </span>
            Users
          </CardTitle>
          <Button size="sm" onClick={() => setShowAdd(true)}>
            <Plus className="h-3.5 w-3.5" strokeWidth={2} /> Add user
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="text-center py-8 font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">Loading…</div>
        ) : (users?.data?.length ?? 0) === 0 ? (
          <div className="text-center py-8 text-sm text-ink-mute">No users yet.</div>
        ) : (
          <div className="rounded-[var(--radius)] border border-line overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Email</TableHead>
                  <TableHead>Role</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {users?.data?.map((u: any) => {
                  const isSelf = u.id === currentUserId;
                  return (
                    <TableRow key={u.id}>
                      <TableCell className="font-medium text-ink">
                        {u.name || "—"}
                        {isSelf && <Badge variant="secondary" className="ml-2">you</Badge>}
                      </TableCell>
                      <TableCell className="text-ink-soft">{u.email}</TableCell>
                      <TableCell>
                        <Badge variant={u.role === "ADMIN" ? "default" : "secondary"}>{u.role.toLowerCase()}</Badge>
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex justify-end gap-0.5">
                          <Button variant="ghost" size="icon" onClick={() => openEdit(u)} className="h-8 w-8" title="Edit">
                            <Pencil className="h-3.5 w-3.5" strokeWidth={1.75} />
                          </Button>
                          <Button variant="ghost" size="icon" onClick={() => { setNewPassword(""); setResetting(u); }} className="h-8 w-8" title="Reset password">
                            <KeyRound className="h-3.5 w-3.5" strokeWidth={1.75} />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() => setDeleting(u)}
                            disabled={isSelf}
                            className="h-8 w-8 hover:text-danger disabled:opacity-30 disabled:hover:text-ink"
                            title={isSelf ? "Cannot delete yourself" : "Delete"}
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
      </CardContent>

      {/* Add user */}
      <Dialog open={showAdd} onClose={() => setShowAdd(false)}>
        <DialogHeader>
          <DialogTitle>Add user</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate({
              email: addForm.email,
              name: addForm.name || null,
              password: addForm.password,
              role: addForm.role,
            });
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Email</Label>
            <Input type="email" value={addForm.email} onChange={(e) => setAddForm({ ...addForm, email: e.target.value })} required />
          </div>
          <div className="space-y-2">
            <Label>Name</Label>
            <Input value={addForm.name} onChange={(e) => setAddForm({ ...addForm, name: e.target.value })} />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Temporary password</Label>
              <Input type="password" value={addForm.password} onChange={(e) => setAddForm({ ...addForm, password: e.target.value })} required minLength={8} />
            </div>
            <div className="space-y-2">
              <Label>Role</Label>
              <Select value={addForm.role} onChange={(e) => setAddForm({ ...addForm, role: e.target.value as Role })}>
                {ROLES.map((r) => (
                  <option key={r} value={r}>{r}</option>
                ))}
              </Select>
            </div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setShowAdd(false)}>Cancel</Button>
            <Button type="submit" disabled={create.isPending}>{create.isPending ? "Creating..." : "Create user"}</Button>
          </div>
        </form>
      </Dialog>

      {/* Edit user */}
      <Dialog open={!!editing} onClose={() => setEditing(null)}>
        <DialogHeader>
          <DialogTitle>Edit user</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (!editing) return;
            update.mutate({
              id: editing.id,
              data: { name: editForm.name || null, role: editForm.role },
            });
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Email</Label>
            <Input value={editing?.email ?? ""} disabled />
          </div>
          <div className="space-y-2">
            <Label>Name</Label>
            <Input value={editForm.name} onChange={(e) => setEditForm({ ...editForm, name: e.target.value })} />
          </div>
          <div className="space-y-2">
            <Label>Role</Label>
            <Select value={editForm.role} onChange={(e) => setEditForm({ ...editForm, role: e.target.value as Role })}>
              {ROLES.map((r) => (
                <option key={r} value={r}>{r}</option>
              ))}
            </Select>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => setEditing(null)}>Cancel</Button>
            <Button type="submit" disabled={update.isPending}>{update.isPending ? "Saving..." : "Save changes"}</Button>
          </div>
        </form>
      </Dialog>

      {/* Reset password */}
      <Dialog open={!!resetting} onClose={() => { setResetting(null); setNewPassword(""); }}>
        <DialogHeader>
          <DialogTitle>Reset password</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (!resetting) return;
            update.mutate({ id: resetting.id, data: { password: newPassword } });
          }}
          className="space-y-4"
        >
          <p className="text-sm text-ink-soft">
            Set a new password for <span className="font-medium text-ink">{resetting?.email}</span>. Share it with them through a secure channel.
          </p>
          <div className="space-y-2">
            <Label>New password</Label>
            <Input type="password" value={newPassword} onChange={(e) => setNewPassword(e.target.value)} required minLength={8} />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button type="button" variant="outline" onClick={() => { setResetting(null); setNewPassword(""); }}>Cancel</Button>
            <Button type="submit" disabled={update.isPending || newPassword.length < 8}>{update.isPending ? "Saving..." : "Reset password"}</Button>
          </div>
        </form>
      </Dialog>

      <ConfirmDialog
        open={!!deleting}
        onClose={() => setDeleting(null)}
        onConfirm={() => deleting && remove.mutate(deleting.id)}
        title="Delete user"
        description={`Delete "${deleting?.email}"? They will lose access immediately. This cannot be undone.`}
        confirmLabel="Delete"
        variant="destructive"
        loading={remove.isPending}
      />
    </Card>
  );
}

function AccountCard({ email, name }: { email?: string | null; name?: string | null }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-3 text-xl">
          <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
            <User className="h-4 w-4 text-accent" strokeWidth={1.75} />
          </span>
          Account
        </CardTitle>
      </CardHeader>
      <CardContent>
        <dl>
          <Row label="Email" value={email || "—"} />
          <Row label="Name" value={name || "—"} />
        </dl>
      </CardContent>
    </Card>
  );
}

function RoleCard({ role }: { role?: string }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-3 text-xl">
          <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
            <Shield className="h-4 w-4 text-accent" strokeWidth={1.75} />
          </span>
          Role &amp; Permissions
        </CardTitle>
      </CardHeader>
      <CardContent>
        <dl>
          <Row label="Role" value={role ? <Badge variant="default">{role.toLowerCase()}</Badge> : "—"} />
        </dl>
      </CardContent>
    </Card>
  );
}
