"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useTenant } from "@/lib/hooks";
import { useMoney } from "@/lib/currency";
import { Plus, Trash2 } from "lucide-react";

interface LineItem {
  productId: string;
  description: string;
  qty: number;
  unitPrice: number;
}

export default function NewInvoicePage() {
  const fmt = useMoney();
  const router = useRouter();
  const { slug } = useTenant();
  const [customerId, setCustomerId] = useState("");
  const [dueDate, setDueDate] = useState("");
  const [notes, setNotes] = useState("");
  const [lineItems, setLineItems] = useState<LineItem[]>([
    { productId: "", description: "", qty: 1, unitPrice: 0 },
  ]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const { data: customersResp } = useQuery({
    queryKey: ["customers", slug, "all"],
    queryFn: () =>
      fetch(`/api/${slug}/customers?limit=500`).then((r) => r.json()),
    enabled: !!slug,
  });

  const { data: productsResp } = useQuery({
    queryKey: ["products", slug, "all"],
    queryFn: () =>
      fetch(`/api/${slug}/products?limit=500`).then((r) => r.json()),
    enabled: !!slug,
  });

  const customers = Array.isArray(customersResp)
    ? customersResp
    : customersResp?.data ?? [];
  const products = Array.isArray(productsResp)
    ? productsResp
    : productsResp?.data ?? [];

  const addLineItem = () => {
    setLineItems([...lineItems, { productId: "", description: "", qty: 1, unitPrice: 0 }]);
  };

  const removeLineItem = (index: number) => {
    if (lineItems.length > 1) {
      setLineItems(lineItems.filter((_, i) => i !== index));
    }
  };

  const updateLineItem = (index: number, field: keyof LineItem, value: any) => {
    const updated = [...lineItems];
    if (field === "productId" && value) {
      const product = products.find((p: any) => p.id === value);
      if (product) {
        updated[index] = {
          ...updated[index],
          productId: value,
          description: product.name,
          unitPrice: Number(product.unitPrice),
        };
      }
    } else {
      (updated[index] as any)[field] = value;
    }
    setLineItems(updated);
  };

  const subtotal = lineItems.reduce((sum, item) => sum + item.qty * item.unitPrice, 0);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setLoading(true);

    try {
      const res = await fetch(`/api/${slug}/invoices`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ customerId, lineItems, dueDate, notes }),
      });

      if (!res.ok) {
        const data = await res.json();
        throw new Error(data.error || "Failed to create invoice");
      }

      const invoice = await res.json();
      router.push(`/${slug}/invoices/${invoice.id}`);
    } catch (err: any) {
      setError(err.message);
      setLoading(false);
    }
  };

  return (
    <div className="max-w-3xl mx-auto">
      <h1 className="text-3xl font-bold mb-6">Create Invoice</h1>

      <form onSubmit={handleSubmit}>
        {error && (
          <div className="rounded-md bg-red-50 p-3 text-sm text-red-600 mb-4">
            {error}
          </div>
        )}

        <Card className="mb-6">
          <CardHeader>
            <CardTitle className="text-lg">Invoice Details</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Customer</Label>
                <Select
                  value={customerId}
                  onChange={(e) => setCustomerId(e.target.value)}
                  required
                >
                  <option value="">Select customer...</option>
                  {customers.map((c: any) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </Select>
              </div>
              <div className="space-y-2">
                <Label>Due Date</Label>
                <Input
                  type="date"
                  value={dueDate}
                  onChange={(e) => setDueDate(e.target.value)}
                  required
                />
              </div>
            </div>
            <div className="space-y-2">
              <Label>Notes</Label>
              <Textarea
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                placeholder="Optional notes..."
                rows={2}
              />
            </div>
          </CardContent>
        </Card>

        <Card className="mb-6">
          <CardHeader>
            <CardTitle className="text-lg">Line Items</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              {lineItems.map((item, idx) => (
                <div key={idx} className="flex gap-2 items-end">
                  <div className="flex-1 space-y-1">
                    {idx === 0 && <Label className="text-xs">Product</Label>}
                    <Select
                      value={item.productId}
                      onChange={(e) => updateLineItem(idx, "productId", e.target.value)}
                    >
                      <option value="">Select product...</option>
                      {products.map((p: any) => (
                        <option key={p.id} value={p.id}>
                          {p.name} ({fmt(p.unitPrice)})
                        </option>
                      ))}
                    </Select>
                  </div>
                  <div className="w-20 space-y-1">
                    {idx === 0 && <Label className="text-xs">Qty</Label>}
                    <Input
                      type="number"
                      min="1"
                      value={item.qty}
                      onChange={(e) =>
                        updateLineItem(idx, "qty", parseInt(e.target.value) || 1)
                      }
                    />
                  </div>
                  <div className="w-32 space-y-1">
                    {idx === 0 && <Label className="text-xs">Unit Price</Label>}
                    <Input
                      type="number"
                      min="0"
                      value={item.unitPrice}
                      onChange={(e) =>
                        updateLineItem(idx, "unitPrice", parseFloat(e.target.value) || 0)
                      }
                    />
                  </div>
                  <div className="w-28 text-right space-y-1">
                    {idx === 0 && <Label className="text-xs">Amount</Label>}
                    <p className="h-10 flex items-center justify-end font-medium text-sm">
                      {fmt(item.qty * item.unitPrice)}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={() => removeLineItem(idx)}
                    disabled={lineItems.length === 1}
                  >
                    <Trash2 className="h-4 w-4 text-gray-400" />
                  </Button>
                </div>
              ))}
            </div>
            <Button
              type="button"
              variant="outline"
              onClick={addLineItem}
              className="mt-4"
            >
              <Plus className="h-4 w-4 mr-2" /> Add Line Item
            </Button>
          </CardContent>
        </Card>

        <Card className="mb-6">
          <CardContent className="pt-6">
            <div className="flex justify-between text-lg font-bold">
              <span>Total</span>
              <span>{fmt(subtotal)}</span>
            </div>
          </CardContent>
        </Card>

        <div className="flex justify-end gap-3">
          <Button
            type="button"
            variant="outline"
            onClick={() => router.back()}
          >
            Cancel
          </Button>
          <Button type="submit" disabled={loading}>
            {loading ? "Creating..." : "Create Invoice"}
          </Button>
        </div>
      </form>
    </div>
  );
}
