"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { PageHeader } from "@/components/page-header";
import { useTenant } from "@/lib/hooks";
import { formatCurrency } from "@/lib/utils";
import { Plus, Trash2 } from "lucide-react";

interface Line {
  productId: string;
  qty: number;
  unitPrice: number;
}

export default function NewSalesOrderPage() {
  const router = useRouter();
  const { slug } = useTenant();
  const [customerId, setCustomerId] = useState("");
  const [expectedDate, setExpectedDate] = useState("");
  const [notes, setNotes] = useState("");
  const [lines, setLines] = useState<Line[]>([
    { productId: "", qty: 1, unitPrice: 0 },
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

  const updateLine = (i: number, field: keyof Line, value: any) => {
    const next = [...lines];
    if (field === "productId" && value) {
      const product = products.find((p: any) => p.id === value);
      next[i] = {
        ...next[i],
        productId: value,
        unitPrice: product ? Number(product.unitPrice) : 0,
      };
    } else {
      (next[i] as any)[field] = value;
    }
    setLines(next);
  };

  const addLine = () =>
    setLines([...lines, { productId: "", qty: 1, unitPrice: 0 }]);
  const removeLine = (i: number) =>
    lines.length > 1 && setLines(lines.filter((_, idx) => idx !== i));

  const subtotal = lines.reduce((s, l) => s + l.qty * l.unitPrice, 0);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      const res = await fetch(`/api/${slug}/sales-orders`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ customerId, expectedDate, notes, lines }),
      });
      if (!res.ok) {
        const d = await res.json();
        throw new Error(d.error || "Failed to create");
      }
      const order = await res.json();
      router.push(`/${slug}/sales/${order.id}`);
    } catch (err: any) {
      setError(err.message);
      setLoading(false);
    }
  };

  return (
    <div className="animate-fade-in max-w-4xl">
      <PageHeader
        eyebrow="Commerce"
        title="New sales order"
        description="Capture an order before it becomes a delivery."
      />

      <form onSubmit={handleSubmit}>
        {error && (
          <div className="rounded-[var(--radius)] border border-danger/20 bg-danger-soft px-4 py-3 text-sm text-danger mb-4">
            {error}
          </div>
        )}

        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-6 mb-5">
          <h3 className="font-display text-xl font-semibold uppercase tracking-tight mb-5">Order details</h3>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 mb-4">
            <div className="space-y-2">
              <Label>Customer</Label>
              <Select
                value={customerId}
                onChange={(e) => setCustomerId(e.target.value)}
                required
              >
                <option value="">Select customer…</option>
                {customers.map((c: any) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </Select>
            </div>
            <div className="space-y-2">
              <Label>Expected delivery</Label>
              <Input
                type="date"
                value={expectedDate}
                onChange={(e) => setExpectedDate(e.target.value)}
              />
            </div>
          </div>
          <div className="space-y-2">
            <Label>Notes</Label>
            <Textarea
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              placeholder="Internal notes (optional)"
              rows={2}
            />
          </div>
        </div>

        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-6 mb-5">
          <h3 className="font-display text-xl font-semibold uppercase tracking-tight mb-5">Line items</h3>
          <div className="space-y-3">
            {lines.map((line, i) => (
              <div key={i} className="flex gap-2 items-end">
                <div className="flex-1 space-y-1">
                  {i === 0 && <Label>Product</Label>}
                  <Select
                    value={line.productId}
                    onChange={(e) =>
                      updateLine(i, "productId", e.target.value)
                    }
                    required
                  >
                    <option value="">Select product…</option>
                    {products.map((p: any) => (
                      <option key={p.id} value={p.id}>
                        {p.name} ({formatCurrency(p.unitPrice)})
                      </option>
                    ))}
                  </Select>
                </div>
                <div className="w-24 space-y-1">
                  {i === 0 && <Label>Qty</Label>}
                  <Input
                    type="number"
                    min="1"
                    value={line.qty}
                    onChange={(e) =>
                      updateLine(i, "qty", parseInt(e.target.value) || 1)
                    }
                  />
                </div>
                <div className="w-32 space-y-1">
                  {i === 0 && <Label>Unit price</Label>}
                  <Input
                    type="number"
                    min="0"
                    step="0.01"
                    value={line.unitPrice}
                    onChange={(e) =>
                      updateLine(i, "unitPrice", parseFloat(e.target.value) || 0)
                    }
                  />
                </div>
                <div className="w-28 text-right space-y-1">
                  {i === 0 && <Label>Amount</Label>}
                  <p className="h-11 flex items-center justify-end font-medium text-sm tabular">
                    {formatCurrency(line.qty * line.unitPrice)}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  onClick={() => removeLine(i)}
                  disabled={lines.length === 1}
                  className="h-11 w-11"
                >
                  <Trash2 className="h-4 w-4" strokeWidth={1.75} />
                </Button>
              </div>
            ))}
          </div>
          <Button
            type="button"
            variant="outline"
            onClick={addLine}
            className="mt-4"
          >
            <Plus className="h-4 w-4" strokeWidth={2} /> Add line
          </Button>
        </div>

        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-6 mb-5">
          <div className="flex justify-between items-baseline">
            <span className="text-[0.7rem] tracking-[0.2em] uppercase text-ink-mute font-medium">
              Subtotal
            </span>
            <span className="font-display text-3xl font-bold uppercase tracking-tight tabular">
              {formatCurrency(subtotal)}
            </span>
          </div>
        </div>

        <div className="flex justify-end gap-3">
          <Button
            type="button"
            variant="outline"
            onClick={() => router.back()}
          >
            Cancel
          </Button>
          <Button type="submit" disabled={loading} size="lg">
            {loading ? "Creating…" : "Create order"}
          </Button>
        </div>
      </form>
    </div>
  );
}
