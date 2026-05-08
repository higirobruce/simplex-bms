"use client";

import { useQuery } from "@tanstack/react-query";
import { Badge } from "@/components/ui/badge";
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
} from "recharts";
import { useTenant } from "@/lib/hooks";
import { formatCurrency } from "@/lib/utils";
import {
  ArrowUpRight,
  ArrowDownRight,
  Bell,
  Search,
  Sparkles,
} from "lucide-react";

interface StatCardProps {
  label: string;
  value: string;
  delta?: { value: string; positive?: boolean };
  caption?: string;
  emphasis?: boolean;
}

function StatCard({ label, value, delta, caption, emphasis }: StatCardProps) {
  return (
    <div
      className={
        emphasis
          ? "rounded-[var(--radius-lg)] bg-accent text-surface p-6 relative overflow-hidden"
          : "rounded-[var(--radius-lg)] border border-line bg-surface p-6"
      }
    >
      {emphasis && (
        <div
          className="absolute -top-10 -right-10 w-44 h-44 rounded-full opacity-20"
          style={{
            background:
              "radial-gradient(circle, var(--gold) 0%, transparent 70%)",
          }}
        />
      )}
      <div className="relative">
        <p
          className={
            emphasis
              ? "text-[0.65rem] tracking-[0.2em] uppercase text-surface/70 font-medium"
              : "text-[0.65rem] tracking-[0.2em] uppercase text-ink-mute font-medium"
          }
        >
          {label}
        </p>
        <p
          className={
            emphasis
              ? "font-serif text-4xl mt-3 leading-none text-surface tabular"
              : "font-serif text-4xl mt-3 leading-none text-ink tabular"
          }
        >
          {value}
        </p>
        {(delta || caption) && (
          <div className="mt-4 flex items-center gap-2">
            {delta && (
              <span
                className={
                  delta.positive
                    ? emphasis
                      ? "inline-flex items-center gap-1 text-xs font-medium text-gold-soft"
                      : "inline-flex items-center gap-1 text-xs font-medium text-success"
                    : "inline-flex items-center gap-1 text-xs font-medium text-danger"
                }
              >
                {delta.positive ? (
                  <ArrowUpRight className="h-3 w-3" strokeWidth={2} />
                ) : (
                  <ArrowDownRight className="h-3 w-3" strokeWidth={2} />
                )}
                {delta.value}
              </span>
            )}
            {caption && (
              <span
                className={
                  emphasis
                    ? "text-xs text-surface/60"
                    : "text-xs text-ink-mute"
                }
              >
                {caption}
              </span>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default function DashboardPage() {
  const { slug } = useTenant();

  const { data: stats, isLoading } = useQuery({
    queryKey: ["dashboard-stats", slug],
    queryFn: () =>
      fetch(`/api/${slug}/dashboard/stats`).then((r) => r.json()),
    enabled: !!slug,
  });

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-96">
        <p className="text-ink-mute font-serif italic text-lg">
          Composing your overview…
        </p>
      </div>
    );
  }

  const statusVariant: Record<
    string,
    "success" | "warning" | "destructive" | "secondary" | "info"
  > = {
    PAID: "success",
    SENT: "info",
    PARTIAL: "warning",
    OVERDUE: "destructive",
    DRAFT: "secondary",
    VOID: "secondary",
  };

  const today = new Date().toLocaleDateString("en-US", {
    weekday: "long",
    month: "long",
    day: "numeric",
  });

  return (
    <div className="animate-fade-in">
      {/* Top utility bar */}
      <div className="flex items-center justify-between mb-10 gap-4">
        <div className="relative flex-1 max-w-md">
          <Search
            className="absolute left-4 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute"
            strokeWidth={1.75}
          />
          <input
            placeholder="Search invoices, products, customers…"
            className="h-11 w-full rounded-full border border-line bg-surface-2/60 pl-11 pr-4 text-sm placeholder:text-ink-mute focus:outline-none focus:bg-surface focus:border-accent/30 focus:ring-4 focus:ring-accent/10 transition-all"
          />
        </div>
        <div className="flex items-center gap-3">
          <button
            className="h-11 w-11 rounded-full border border-line bg-surface flex items-center justify-center hover:bg-surface-2 transition-colors"
            aria-label="Notifications"
          >
            <Bell className="h-4 w-4 text-ink-soft" strokeWidth={1.75} />
          </button>
          <div className="h-11 px-4 rounded-full border border-line bg-surface flex items-center gap-2.5">
            <div className="h-7 w-7 rounded-full bg-accent text-surface flex items-center justify-center">
              <span className="font-serif italic text-sm leading-none -mt-0.5">
                B
              </span>
            </div>
            <span className="text-sm text-ink hidden sm:inline">Bruce</span>
          </div>
        </div>
      </div>

      {/* Greeting */}
      <div className="mb-10 flex items-end justify-between flex-wrap gap-4">
        <div>
          <p className="text-[0.7rem] tracking-[0.22em] uppercase text-gold font-medium mb-3 flex items-center gap-2">
            <Sparkles className="h-3.5 w-3.5" strokeWidth={1.75} />
            {today}
          </p>
          <h1 className="font-serif text-5xl sm:text-6xl leading-[1.05] text-ink">
            Good morning,
            <br />
            <span className="italic">welcome back.</span>
          </h1>
          <p className="mt-4 text-ink-soft max-w-xl leading-relaxed">
            A considered glance at your atelier — revenue, receivables and the
            quiet pulse of inventory.
          </p>
        </div>
      </div>

      {/* Metrics */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-10">
        <StatCard
          label="Revenue · MTD"
          value={formatCurrency(stats?.revenueMTD || 0)}
          delta={{ value: "+12.4%", positive: true }}
          caption="vs last month"
          emphasis
        />
        <StatCard
          label="Expenses · MTD"
          value={formatCurrency(stats?.expensesMTD || 0)}
          delta={{ value: "−4.1%", positive: true }}
          caption="vs last month"
        />
        <StatCard
          label="Outstanding"
          value={formatCurrency(stats?.outstanding || 0)}
          delta={{ value: "+2.8%", positive: false }}
          caption="receivables"
        />
        <StatCard
          label="Low Stock"
          value={String(stats?.lowStockCount || 0)}
          caption="items need restock"
        />
      </div>

      {/* Lower grid */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-5">
        <div className="lg:col-span-2 rounded-[var(--radius-lg)] border border-line bg-surface p-7">
          <div className="flex items-start justify-between mb-6">
            <div>
              <h2 className="font-serif text-2xl text-ink leading-tight">
                Revenue movement
              </h2>
              <p className="text-sm text-ink-soft mt-1">
                Daily inflow over the past period
              </p>
            </div>
            <div className="flex gap-1 p-1 rounded-full bg-surface-2 border border-line">
              {["7d", "30d", "90d"].map((r, i) => (
                <button
                  key={r}
                  className={
                    i === 1
                      ? "px-3 py-1 rounded-full text-xs bg-surface text-ink shadow-sm border border-line"
                      : "px-3 py-1 rounded-full text-xs text-ink-soft hover:text-ink"
                  }
                >
                  {r}
                </button>
              ))}
            </div>
          </div>
          <ResponsiveContainer width="100%" height={280}>
            <AreaChart
              data={stats?.trendData || []}
              margin={{ top: 8, right: 0, left: -12, bottom: 0 }}
            >
              <defs>
                <linearGradient id="revFill" x1="0" y1="0" x2="0" y2="1">
                  <stop
                    offset="0%"
                    stopColor="var(--accent)"
                    stopOpacity={0.18}
                  />
                  <stop
                    offset="100%"
                    stopColor="var(--accent)"
                    stopOpacity={0}
                  />
                </linearGradient>
              </defs>
              <CartesianGrid
                strokeDasharray="2 4"
                stroke="var(--line)"
                vertical={false}
              />
              <XAxis
                dataKey="date"
                fontSize={11}
                stroke="var(--ink-mute)"
                tickLine={false}
                axisLine={false}
              />
              <YAxis
                fontSize={11}
                stroke="var(--ink-mute)"
                tickLine={false}
                axisLine={false}
                tickFormatter={(v) => `${(v / 1000).toFixed(0)}k`}
              />
              <Tooltip
                contentStyle={{
                  background: "var(--surface)",
                  border: "1px solid var(--line)",
                  borderRadius: "12px",
                  fontSize: "12px",
                  boxShadow:
                    "0 8px 24px -12px rgba(21,17,13,0.16)",
                }}
                formatter={(value) => [
                  formatCurrency(Number(value)),
                  "Revenue",
                ]}
              />
              <Area
                type="monotone"
                dataKey="revenue"
                stroke="var(--accent)"
                strokeWidth={2}
                fill="url(#revFill)"
                dot={false}
              />
            </AreaChart>
          </ResponsiveContainer>
        </div>

        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-7">
          <div className="flex items-center justify-between mb-5">
            <h2 className="font-serif text-2xl text-ink leading-tight">
              Recent invoices
            </h2>
            <span className="text-[0.65rem] tracking-[0.18em] uppercase text-ink-mute">
              Latest
            </span>
          </div>
          <div className="-mx-2">
            {stats?.recentInvoices?.map((inv: any, idx: number) => (
              <div
                key={inv.id}
                className={
                  "flex items-center justify-between px-2 py-3.5 " +
                  (idx > 0 ? "border-t border-line/70" : "")
                }
              >
                <div className="min-w-0">
                  <p className="font-medium text-sm text-ink truncate">
                    {inv.invoiceNo}
                  </p>
                  <p className="text-xs text-ink-mute truncate mt-0.5">
                    {inv.customer?.name}
                  </p>
                </div>
                <div className="text-right ml-3 flex-shrink-0">
                  <p className="font-medium text-sm text-ink tabular">
                    {formatCurrency(inv.total)}
                  </p>
                  <div className="mt-1 flex justify-end">
                    <Badge variant={statusVariant[inv.status] || "secondary"}>
                      {inv.status?.toLowerCase()}
                    </Badge>
                  </div>
                </div>
              </div>
            ))}
            {(!stats?.recentInvoices ||
              stats.recentInvoices.length === 0) && (
              <p className="text-sm text-ink-mute italic font-serif text-center py-12">
                No invoices yet — your ledger awaits.
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
