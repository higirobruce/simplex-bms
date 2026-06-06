"use client";

import { apiCall } from "@/lib/fetcher";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useParams, useRouter } from "next/navigation";
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
import { formatDate } from "@/lib/utils";
import { useMoney } from "@/lib/currency";
import { ArrowLeft, Plus, Check, Inbox, X } from "lucide-react";

const statusVariant: Record<string, any> = {
  DRAFT: "secondary",
  CONFIRMED: "info",
  RECEIVED: "success",
  CANCELLED: "destructive",
  APPROVED: "success",
};
export default function PurchaseOrderDetailPage() {
  const fmt = useMoney();
  const { id } = useParams<{ id: string }>();
  const router = useRouter();
  const { slug } = useTenant();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [showGRN, setShowGRN] = useState(false);
  const [grnLocation, setGrnLocation] = useState("");
  const [grnNotes, setGrnNotes] = useState("");
  const [grnLines, setGrnLines] = useState<Record<string, number>>({});
  const [confirmingApproveId, setConfirmingApproveId] = useState<string | null>(
    null
  );

  const { data: order, isLoading } = useQuery({
    queryKey: ["purchase-order", slug, id],
    queryFn: () =>
      fetch(`/api/${slug}/purchase-orders/${id}`).then((r) => r.json()),
    enabled: !!slug && !!id,
  });

  const { data: locations = [] } = useQuery<any[]>({
    queryKey: ["locations", slug],
    queryFn: () => fetch(`/api/${slug}/locations`).then((r) => r.json()),
    enabled: !!slug,
  });

  const confirmOrder = useMutation({
    mutationFn: () =>
      apiCall(`/api/${slug}/purchase-orders/${id}/confirm`, { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["purchase-order", slug, id] });
      toast.success("Order confirmed");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const cancelOrder = useMutation({
    mutationFn: () =>
      apiCall(`/api/${slug}/purchase-orders/${id}/cancel`, { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["purchase-order", slug, id] });
      toast.success("Order cancelled");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const createGRN = useMutation({
    mutationFn: () => {
      const lines = Object.entries(grnLines)
        .filter(([, qty]) => qty > 0)
        .map(([orderLineId, qty]) => {
          const ol = order.lines.find((l: any) => l.id === orderLineId);
          return { orderLineId, productId: ol.productId, qty };
        });
      return apiCall(`/api/${slug}/grns`, {
        method: "POST",
        body: JSON.stringify({
          orderId: id,
          locationId: grnLocation,
          notes: grnNotes,
          lines,
        }),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["purchase-order", slug, id] });
      setShowGRN(false);
      setGrnLines({});
      setGrnLocation("");
      setGrnNotes("");
      toast.success("Goods receipt drafted");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const approveGRN = useMutation({
    mutationFn: (grnId: string) =>
      apiCall(`/api/${slug}/grns/${grnId}/approve`, { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["purchase-order", slug, id] });
      queryClient.invalidateQueries({ queryKey: ["products", slug] });
      setConfirmingApproveId(null);
      toast.success("Goods received · stock updated");
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
        onClick={() => router.push(`/${slug}/procurement`)}
        className="flex items-center gap-2 text-sm text-ink-soft hover:text-ink mb-4"
      >
        <ArrowLeft className="h-4 w-4" strokeWidth={1.75} /> Back to procurement
      </button>

      <PageHeader
        eyebrow={`Purchase Order · ${order.orderNo}`}
        title={order.vendor.name}
        description={`Placed ${formatDate(order.orderDate)}${
          order.expectedDate
            ? ` · Expected ${formatDate(order.expectedDate)}`
            : ""
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
              <Button onClick={() => setShowGRN(true)} size="lg">
                <Inbox className="h-4 w-4" strokeWidth={2} /> New goods receipt
              </Button>
            )}
          </div>
        }
      />

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
                Received
              </th>
              <th className="px-6 py-3 text-right text-[0.65rem] tracking-[0.18em] uppercase font-medium text-ink-mute">
                Unit cost
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
                      l.qtyReceived >= l.qty
                        ? "text-success font-medium"
                        : l.qtyReceived > 0
                        ? "text-warning"
                        : "text-ink-mute"
                    }
                  >
                    {l.qtyReceived}
                  </span>
                </td>
                <td className="px-6 py-4 text-right tabular">
                  {fmt(l.unitPrice)}
                </td>
                <td className="px-6 py-4 text-right tabular font-medium">
                  {fmt(l.amount)}
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
                {fmt(order.total)}
              </td>
            </tr>
          </tfoot>
        </table>
      </div>

      <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-6 mb-6">
        <h3 className="font-display text-xl font-semibold uppercase tracking-tight mb-4">Goods receipts</h3>
        {order.receipts.length === 0 ? (
          <p className="font-mono text-xs uppercase tracking-wider text-ink-mute">
            No receipts yet.
          </p>
        ) : (
          <div className="space-y-3">
            {order.receipts.map((grn: any) => (
              <div
                key={grn.id}
                className="rounded-[var(--radius)] border border-line p-4 flex items-start justify-between gap-4"
              >
                <div className="flex-1">
                  <div className="flex items-center gap-3 mb-1">
                    <p className="font-medium text-ink tabular">{grn.noteNo}</p>
                    <Badge variant={statusVariant[grn.status] || "secondary"}>
                      {grn.status?.toLowerCase()}
                    </Badge>
                  </div>
                  <p className="text-xs text-ink-mute">
                    {grn.location?.name} · {formatDate(grn.createdAt)} ·{" "}
                    {grn.lines.length} line{grn.lines.length === 1 ? "" : "s"}
                  </p>
                </div>
                {grn.status === "DRAFT" && (
                  <Button
                    onClick={() => setConfirmingApproveId(grn.id)}
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

      <Dialog open={showGRN} onClose={() => setShowGRN(false)}>
        <DialogHeader>
          <DialogTitle>New goods receipt</DialogTitle>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            createGRN.mutate();
          }}
          className="space-y-4"
        >
          <div className="space-y-2">
            <Label>Destination warehouse</Label>
            <Select
              value={grnLocation}
              onChange={(e) => setGrnLocation(e.target.value)}
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
            <Label>Lines received</Label>
            <div className="space-y-2 rounded-[var(--radius)] border border-line p-3">
              {order.lines.map((l: any) => {
                const remaining = l.qty - l.qtyReceived;
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
                      value={grnLines[l.id] ?? ""}
                      onChange={(e) =>
                        setGrnLines({
                          ...grnLines,
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
              value={grnNotes}
              onChange={(e) => setGrnNotes(e.target.value)}
              placeholder="Optional"
              rows={2}
            />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setShowGRN(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={createGRN.isPending}>
              {createGRN.isPending ? "Creating…" : "Create draft"}
            </Button>
          </div>
        </form>
      </Dialog>

      <ConfirmDialog
        open={!!confirmingApproveId}
        onClose={() => setConfirmingApproveId(null)}
        onConfirm={() =>
          confirmingApproveId && approveGRN.mutate(confirmingApproveId)
        }
        title="Approve goods receipt"
        description="Approving will add the received items to the destination warehouse stock. This cannot be undone."
        confirmLabel="Approve"
        loading={approveGRN.isPending}
      />
    </div>
  );
}
