"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useParams } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useToast } from "@/components/ui/toast";
import { useTenant } from "@/lib/hooks";
import { formatDate } from "@/lib/utils";
import { useMoney } from "@/lib/currency";

function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, { ...opts, headers: { "Content-Type": "application/json", ...opts?.headers } }).then(async (r) => {
    if (!r.ok) { const d = await r.json(); throw new Error(d.error || "Request failed"); }
    return r.json();
  });
}

const statusVariant: Record<string, "success" | "warning" | "destructive" | "secondary" | "outline"> = {
  PAID: "success",
  SENT: "secondary",
  PARTIAL: "warning",
  OVERDUE: "destructive",
  DRAFT: "outline",
  VOID: "secondary",
};

export default function InvoiceDetailPage() {
  const fmt = useMoney();
  const { slug } = useTenant();
  const params = useParams();
  const invoiceId = params?.id as string;
  const queryClient = useQueryClient();
  const toast = useToast();
  const [showPayment, setShowPayment] = useState(false);
  const [showVoid, setShowVoid] = useState(false);
  const [paymentForm, setPaymentForm] = useState({
    amount: "",
    method: "CASH",
    notes: "",
  });

  const { data: invoice, isLoading } = useQuery({
    queryKey: ["invoice", invoiceId],
    queryFn: () => fetch(`/api/${slug}/invoices/${invoiceId}`).then((r) => r.json()),
    enabled: !!slug && !!invoiceId,
  });

  const recordPayment = useMutation({
    mutationFn: (data: any) => apiCall(`/api/${slug}/invoices/${invoiceId}/payments`, { method: "POST", body: JSON.stringify(data) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["invoice", invoiceId] });
      queryClient.invalidateQueries({ queryKey: ["invoices", slug] });
      setShowPayment(false);
      setPaymentForm({ amount: "", method: "CASH", notes: "" });
      toast.success("Payment recorded");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const statusActions = useMutation({
    mutationFn: (action: string) => apiCall(`/api/${slug}/invoices/${invoiceId}/status`, { method: "POST", body: JSON.stringify({ action }) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["invoice", invoiceId] });
      queryClient.invalidateQueries({ queryKey: ["invoices", slug] });
      setShowVoid(false);
      toast.success("Invoice status updated");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const handlePayment = (e: React.FormEvent) => {
    e.preventDefault();
    recordPayment.mutate({
      amount: parseFloat(paymentForm.amount),
      method: paymentForm.method,
      notes: paymentForm.notes,
    });
  };

  if (isLoading) {
    return <div className="text-center py-8 text-gray-500">Loading invoice...</div>;
  }

  if (!invoice) {
    return <div className="text-center py-8 text-gray-500">Invoice not found</div>;
  }

  const balance = Number(invoice.total) - Number(invoice.amountPaid);

  return (
    <div className="max-w-3xl mx-auto">
      <div className="flex justify-between items-start mb-6">
        <div>
          <h1 className="text-3xl font-bold">{invoice.invoiceNo}</h1>
          <p className="text-gray-500 mt-1">{invoice.customer?.name}</p>
        </div>
        <div className="flex items-center gap-2 flex-wrap">
          <Badge variant={statusVariant[invoice.status] || "secondary"} className="text-sm px-3 py-1">
            {invoice.status}
          </Badge>
          {invoice.status === "DRAFT" && (
            <>
              <Button size="sm" onClick={() => statusActions.mutate("send")} disabled={statusActions.isPending}>Send Invoice</Button>
              <Button size="sm" variant="outline" onClick={() => setShowVoid(true)}>Void</Button>
            </>
          )}
          {(invoice.status === "SENT" || invoice.status === "PARTIAL" || invoice.status === "OVERDUE") && (
            <>
              <Button size="sm" onClick={() => setShowPayment(true)}>Record Payment</Button>
              {invoice.status !== "OVERDUE" && (
                <Button size="sm" variant="outline" onClick={() => statusActions.mutate("mark_overdue")} disabled={statusActions.isPending}>Mark Overdue</Button>
              )}
              <Button size="sm" variant="outline" onClick={() => setShowVoid(true)}>Void</Button>
            </>
          )}
          {invoice.status === "VOID" && (
            <Button size="sm" variant="outline" onClick={() => statusActions.mutate("reopen")} disabled={statusActions.isPending}>Reopen</Button>
          )}
        </div>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-6">
        <Card>
          <CardContent className="pt-4 pb-4">
            <p className="text-xs text-gray-500">Total</p>
            <p className="text-lg font-bold">{fmt(invoice.total)}</p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <p className="text-xs text-gray-500">Paid</p>
            <p className="text-lg font-bold text-green-600">
              {fmt(invoice.amountPaid)}
            </p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <p className="text-xs text-gray-500">Balance</p>
            <p className="text-lg font-bold text-red-600">
              {fmt(balance)}
            </p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <p className="text-xs text-gray-500">Due Date</p>
            <p className="text-lg font-bold">{formatDate(invoice.dueDate)}</p>
          </CardContent>
        </Card>
      </div>

      <Card className="mb-6">
        <CardHeader>
          <CardTitle className="text-lg">Line Items</CardTitle>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Item</TableHead>
                <TableHead className="text-right">Qty</TableHead>
                <TableHead className="text-right">Unit Price</TableHead>
                <TableHead className="text-right">Amount</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {invoice.lineItems?.map((item: any) => (
                <TableRow key={item.id}>
                  <TableCell>
                    {item.product?.name || item.description || "—"}
                    {item.product?.sku && (
                      <span className="text-xs text-gray-400 ml-2">
                        ({item.product.sku})
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">{item.qty}</TableCell>
                  <TableCell className="text-right">
                    {fmt(item.unitPrice)}
                  </TableCell>
                  <TableCell className="text-right font-medium">
                    {fmt(item.amount)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {invoice.payments?.length > 0 && (
        <Card className="mb-6">
          <CardHeader>
            <CardTitle className="text-lg">Payment History</CardTitle>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Date</TableHead>
                  <TableHead>Method</TableHead>
                  <TableHead className="text-right">Amount</TableHead>
                  <TableHead>Notes</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {invoice.payments.map((payment: any) => (
                  <TableRow key={payment.id}>
                    <TableCell>{formatDate(payment.date)}</TableCell>
                    <TableCell>
                      <Badge variant="outline">{payment.method}</Badge>
                    </TableCell>
                    <TableCell className="text-right font-medium">
                      {fmt(payment.amount)}
                    </TableCell>
                    <TableCell>{payment.notes || "—"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      <ConfirmDialog
        open={showVoid}
        onClose={() => setShowVoid(false)}
        onConfirm={() => statusActions.mutate("void")}
        title="Void Invoice"
        description="Are you sure you want to void this invoice? This action marks it as cancelled."
        confirmLabel="Void Invoice"
        variant="destructive"
        loading={statusActions.isPending}
      />

      <Dialog open={showPayment} onClose={() => setShowPayment(false)}>
        <DialogHeader>
          <DialogTitle>Record Payment</DialogTitle>
        </DialogHeader>
        <form onSubmit={handlePayment} className="space-y-4">
          <div className="space-y-2">
            <Label>Amount (Balance: {fmt(balance)})</Label>
            <Input
              type="number"
              min="0"
              max={balance}
              value={paymentForm.amount}
              onChange={(e) =>
                setPaymentForm({ ...paymentForm, amount: e.target.value })
              }
              placeholder="0"
              required
            />
          </div>
          <div className="space-y-2">
            <Label>Payment Method</Label>
            <Select
              value={paymentForm.method}
              onChange={(e) =>
                setPaymentForm({ ...paymentForm, method: e.target.value })
              }
            >
              <option value="CASH">Cash</option>
              <option value="MOBILE_MONEY">Mobile Money</option>
              <option value="BANK">Bank Transfer</option>
              <option value="CHECK">Check</option>
              <option value="OTHER">Other</option>
            </Select>
          </div>
          <div className="space-y-2">
            <Label>Notes</Label>
            <Input
              value={paymentForm.notes}
              onChange={(e) =>
                setPaymentForm({ ...paymentForm, notes: e.target.value })
              }
              placeholder="Optional notes"
            />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setShowPayment(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={recordPayment.isPending}>
              {recordPayment.isPending ? "Recording..." : "Record Payment"}
            </Button>
          </div>
        </form>
      </Dialog>
    </div>
  );
}
