"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { useTenant } from "@/lib/hooks";
import { PageHeader } from "@/components/page-header";
import { formatCurrency, formatDate } from "@/lib/utils";
import { ArrowLeft, Plus, Check, Truck, X } from "lucide-react";

const statusVariant: Record<string, any> = {
  DRAFT: "secondary",
  CONFIRMED: "info",
  FULFILLED: "success",
  CANCELLED: "destructive",
  APPROVED: "success",
};

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

export default function SalesOrderDetailPage() {
  const { id } = useParams<{ id: string }>();
  const router = useRouter();
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [showDN, setShowDN] = useState(false);
  const [dnLocation, setDnLocation] = useState("");
  const [dnNotes, setDnNotes] = useState("");
  const [dnLines, setDnLines] = useState<Record<string, number>>({});
  const [confirmingApproveId, setConfirmingApproveId] = useState<string | null>(
    null
  );

  const { data: order, isLoading } = useQuery({
    queryKey: ["sales-order", slug, id],
    queryFn: () =>
      fetch(`/api/${slug}/sales-orders/${id}`).then((r) => r.json()),
    enabled: !!slug && !!id,
  });

  const { data: locations = [] } = useQuery<any[]>({
    queryKey: ["locations", slug],
    queryFn: () => fetch(`/api/${slug}/locations`).then((r) => r.json()),
    enabled: !!slug,
  });

  const confirmOrder = useMutation({
    mutationFn: () =>
      apiCall(`/api/${slug}/sales-orders/${id}/confirm`, { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sales-order", slug, id] });
      toast.success("Order confirmed");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const cancelOrder = useMutation({
    mutationFn: () =>
      apiCall(`/api/${slug}/sales-orders/${id}/cancel`, { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sales-order", slug, id] });
      toast.success("Order cancelled");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const createDN = useMutation({
    mutationFn: () => {
      const lines = Object.entries(dnLines)
        .filter(([, qty]) => qty > 0)
        .map(([orderLineId, qty]) => {
          const ol = order.lines.find((l: any) => l.id === orderLineId);
          return { orderLineId, productId: ol.productId, qty };
        });
      return apiCall(`/api/${slug}/delivery-notes`, {
        method: "POST",
        body: JSON.stringify({
          orderId: id,
          locationId: dnLocation,
          notes: dnNotes,
          lines,
        }),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sales-order", slug, id] });
      setShowDN(false);
      setDnLines({});
      setDnLocation("");
      setDnNotes("");
      toast.success("Delivery note created");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const approveDN = useMutation({
    mutationFn: (dnId: string) =>
      apiCall(`/api/${slug}/delivery-notes/${dnId}/approve`, { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sales-order", slug, id] });
      setConfirmingApproveId(null);
      toast.success("Delivery approved · stock updated · invoice drafted");
    },
    onError: (e: Error) => {
      setConfirmingApproveId(null);
      toast.error(e.message);
    },
  });

  if (isLoading || !order) {
    return (
      <div className="text-center py-16 font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">
        Loading…
      </div>
    );
  }

  return (
    <div className="animate-fade-in">
      <button
        onClick={() => router.push(`/${slug}/sales`)}
        className="flex items-center gap-2 text-sm text-ink-soft hover:text-ink mb-4"
      >
        <ArrowLeft className="h-4 w-4" strokeWidth={1.75} /> Back to sales
      </button>

      <PageHeader
        eyebrow={`Sales Order · ${order.orderNo}`}
        title={order.customer.name}
        description={`Placed ${formatDate(order.orderDate)}${
          order.expectedDate ? ` · Expected ${formatDate(order.expectedDate)}` : ""
        }`}
        actions={
          <div className="flex items-center gap-3">
            <Badge variant={statusVariant[order.status] || "secondary"}>
              {order.status?.toLowerCase()}
            </Badge>
            {order.status === "DRAFT" && (
              <>
                <Button
                  variant="outline"
                  onClick={() => cancelOrder.mutate()}
                  disabled={cancelOrder.isPending}
                >
                  <X className="h-4 w-4" strokeWidth={2} /> Cancel
                </Button>
                <Button
                  onClick={() => confirmOrder.mutate()}
                  disabled={confirmOrder.isPending}
                >
                  <Check className="h-4 w-4" strokeWidth={2} /> Confirm
                </Button>
              </>
            )}
            {order.status === "CONFIRMED" && (
              <Button onClick={() => setShowDN(true)} size="lg">
                <Truck className="h-4 w-4" strokeWidth={2} /> New delivery note
              </Button>
            )}
          </div>
        }
      />

      {/* Lines */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-surface overflow-hidden mb-6">
        <div className="px-6 py-4 border-b border-line">
          <h3 className="font-display text-xl font-semibold uppercase tracking-tight">Lines</h3>
        </div>
        <table className="w-full text-sm">
          <thead className="border-b border-line">
            <tr>
              <th className="px-6 py-3 text-left text-[0.65rem] tracking-[0.18em] uppercase font-medium text-ink-mute">
                Product
              </th>
              <th className="px-6 py-3 text-right text-[0.65rem] tracking-[0.18em] uppercase font-medium text-ink-mute">
                Qty
              </th>
              <th className="px-6 py-3 text-right text-[0.65rem] tracking-[0.18em] uppercase font-medium text-ink-mute">
                Delivered
              </th>
              <th className="px-6 py-3 text-right text-[0.65rem] tracking-[0.18em] uppercase font-medium text-ink-mute">
                Unit price
              </th>
              <th className="px-6 py-3 text-right text-[0.65rem] tracking-[0.18em] uppercase font-medium text-ink-mute">
                Amount
              </th>
            </tr>
          </thead>
          <tbody>
            {order.lines.map((l: any) => (
              <tr key={l.id} className="border-b border-line/70 last:border-0">
                <td className="px-6 py-4">
                  <p className="font-medium text-ink">{l.product.name}</p>
                  <p className="text-xs text-ink-mute font-mono">
                    {l.product.sku}
                  </p>
                </td>
                <td className="px-6 py-4 text-right tabular">{l.qty}</td>
                <td className="px-6 py-4 text-right tabular">
                  <span
                    className={
                      l.qtyDelivered >= l.qty
                        ? "text-success font-medium"
                        : l.qtyDelivered > 0
                        ? "text-warning"
                        : "text-ink-mute"
                    }
                  >
                    {l.qtyDelivered}
                  </span>
                </td>
                <td className="px-6 py-4 text-right tabular">
                  {formatCurrency(l.unitPrice)}
                </td>
                <td className="px-6 py-4 text-right tabular font-medium">
                  {formatCurrency(l.amount)}
                </td>
              </tr>
            ))}
          </tbody>
          <tfoot>
            <tr>
              <td colSpan={4} className="px-6 py-4 text-right text-ink-soft">
                Total
              </td>
              <td className="px-6 py-4 text-right font-display text-2xl font-bold uppercase tracking-tight tabular">
                {formatCurrency(order.total)}
              </td>
            </tr>
          </tfoot>
        </table>
      </div>

      {/* Delivery notes */}
      <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-6 mb-6">
        <h3 className="font-display text-xl font-semibold uppercase tracking-tight mb-4">Delivery notes</h3>
        {order.deliveryNotes.length === 0 ? (
          <p className="font-mono text-xs uppercase tracking-wider text-ink-mute">
            No deliveries yet.
          </p>
        ) : (
          <div className="space-y-3">
            {order.deliveryNotes.map((dn: any) => (
              <div
                key={dn.id}
                className="rounded-[var(--radius)] border border-line p-4 flex items-start justify-between gap-4"
              >
                <div className="flex-1">
                  <div className="flex items-center gap-3 mb-1">
                    <p className="font-medium text-ink tabular">{dn.noteNo}</p>
                    <Badge variant={statusVariant[dn.status] || "secondary"}>
                      {dn.status?.toLowerCase()}
                    </Badge>
                  </div>
                  <p className="text-xs text-ink-mute">
                    {dn.location?.name} · {formatDate(dn.createdAt)} ·{" "}
                    {dn.lines.length} line{dn.lines.length === 1 ? "" : "s"}
                  </p>
                  {dn.invoice && (
                    <p className="text-xs text-ink-soft mt-1">
                      Invoice{" "}
                      <Link
                        href={`/${slug}/invoices/${dn.invoice.id}`}
                        className="text-accent hover:underline"
                      >
                        {dn.invoice.invoiceNo}
                      </Link>{" "}
                      · {dn.invoice.status?.toLowerCase()}
                    </p>
                  )}
                </div>
                {dn.status === "DRAFT" && (
                  <Button
                    onClick={() => setConfirmingApproveId(dn.id)}
                    size="sm"
                  >
                    <Check className="h-3.5 w-3.5" strokeWidth={2} /> Approve
                  </Button>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* New DN dialog */}
      <Dialog open={showDN} onClose={() => setShowDN(false)}>
        <DialogHeader>
          <DialogTitle>New delivery note</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            createDN.mutate();
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Source warehouse</Label>
            <Select
              value={dnLocation}
              onChange={(e) => setDnLocation(e.target.value)}
              required
            >
              <option value="">Select warehouse…</option>
              {locations.map((l: any) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                  {l.code ? ` (${l.code})` : ""}
                </option>
              ))}
            </Select>
          </div>
          <div className="space-y-2">
            <Label>Lines to deliver</Label>
            <div className="space-y-2 rounded-[var(--radius)] border border-line p-3">
              {order.lines.map((l: any) => {
                const remaining = l.qty - l.qtyDelivered;
                return (
                  <div
                    key={l.id}
                    className="flex items-center gap-3 text-sm"
                  >
                    <div className="flex-1">
                      <p className="text-ink">{l.product.name}</p>
                      <p className="text-xs text-ink-mute">
                        Remaining: {remaining}
                      </p>
                    </div>
                    <Input
                      type="number"
                      min="0"
                      max={remaining}
                      placeholder="0"
                      value={dnLines[l.id] ?? ""}
                      onChange={(e) =>
                        setDnLines({
                          ...dnLines,
                          [l.id]: parseInt(e.target.value) || 0,
                        })
                      }
                      disabled={remaining === 0}
                      className="w-24 h-9"
                    />
                  </div>
                );
              })}
            </div>
          </div>
          <div className="space-y-2">
            <Label>Notes</Label>
            <Textarea
              value={dnNotes}
              onChange={(e) => setDnNotes(e.target.value)}
              placeholder="Optional"
              rows={2}
            />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setShowDN(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={createDN.isPending}>
              {createDN.isPending ? "Creating…" : "Create draft"}
            </Button>
          </div>
        </form>
      </Dialog>

      <ConfirmDialog
        open={!!confirmingApproveId}
        onClose={() => setConfirmingApproveId(null)}
        onConfirm={() =>
          confirmingApproveId && approveDN.mutate(confirmingApproveId)
        }
        title="Approve delivery note"
        description="Approving will deduct stock from the source warehouse and create a draft invoice. This cannot be undone."
        confirmLabel="Approve"
        loading={approveDN.isPending}
      />
    </div>
  );
}
