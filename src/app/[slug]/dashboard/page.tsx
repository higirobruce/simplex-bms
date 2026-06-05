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
import { useMoney } from "@/lib/currency";
import {
  ArrowUpRight,
  ArrowDownRight,
  Bell,
  Search,
  Activity,
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
          ? "brushed p-5 relative"
          : "rounded-[var(--radius-lg)] border border-line bg-surface p-5"
      }
    >
      {emphasis && (
        <div
          className="absolute top-0 right-0 h-1 w-full"
          style={{ background: "var(--gold)" }}
        />
      )}
      <div className="relative">
        <p
          className={
            "font-mono text-[0.62rem] tracking-[0.22em] uppercase font-semibold " +
            (emphasis ? "text-gold" : "text-ink-mute")
          }
        >
          {label}
        </p>
        <p
          className={
            "font-display font-bold text-4xl mt-3 leading-none tabular tracking-tight " +
            (emphasis ? "text-surface" : "text-ink")
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
                      ? "inline-flex items-center gap-1 text-xs font-mono font-semibold text-gold"
                      : "inline-flex items-center gap-1 text-xs font-mono font-semibold text-success"
                    : "inline-flex items-center gap-1 text-xs font-mono font-semibold text-danger"
                }
              >
                {delta.positive ? (
                  <ArrowUpRight className="h-3 w-3" strokeWidth={2.5} />
                ) : (
                  <ArrowDownRight className="h-3 w-3" strokeWidth={2.5} />
                )}
                {delta.value}
              </span>
            )}
            {caption && (
              <span
                className={
                  "font-mono text-[0.65rem] uppercase tracking-wider " +
                  (emphasis ? "text-surface/60" : "text-ink-mute")
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
  const fmt = useMoney();

  const { data: stats, isLoading } = useQuery({
    queryKey: ["dashboard-stats", slug],
    queryFn: () =>
      fetch(`/api/${slug}/dashboard/stats`).then((r) => r.json()),
    enabled: !!slug,
  });

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-96">
        <p className="font-mono text-xs uppercase tracking-[0.2em] text-ink-mute">
          Loading inventory…
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

  const today = new Date()
    .toLocaleDateString("en-US", {
      weekday: "short",
      month: "short",
      day: "numeric",
    })
    .toUpperCase();

  return (
    <div className="animate-fade-in">
      {/* Top utility bar */}
      <div className="flex items-center justify-between mb-8 gap-4">
        <div className="relative flex-1 max-w-md">
          <Search
            className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute"
            strokeWidth={2}
          />
          <input
            placeholder="Lookup SKU, customer, vendor…"
            className="h-10 w-full rounded-[var(--radius)] border border-line bg-surface pl-10 pr-4 text-sm placeholder:text-ink-mute focus:outline-none focus:border-gold focus:ring-2 focus:ring-gold/30 transition-all"
          />
        </div>
        <div className="flex items-center gap-3">
          <button
            className="h-10 w-10 rounded-[var(--radius)] border border-line bg-surface flex items-center justify-center hover:bg-surface-2 transition-colors"
            aria-label="Notifications"
          >
            <Bell className="h-4 w-4 text-ink-soft" strokeWidth={2} />
          </button>
          <div className="h-10 px-3 rounded-[var(--radius)] border border-line bg-surface flex items-center gap-2.5">
            <div className="stencil-block h-6 w-6 text-[0.7rem]">B</div>
            <span className="text-sm text-ink hidden sm:inline font-medium">
              Bruce
            </span>
          </div>
        </div>
      </div>

      {/* Status strip */}
      <div className="mb-6 flex items-center justify-between flex-wrap gap-3 px-4 py-3 rounded-[var(--radius)] border border-line bg-surface-2">
        <div className="flex items-center gap-3">
          <span className="flex h-2 w-2 rounded-full bg-success animate-pulse" />
          <span className="font-mono text-[0.7rem] tracking-[0.18em] uppercase font-semibold text-ink">
            Online
          </span>
          <span className="text-line-strong">/</span>
          <span className="font-mono text-[0.7rem] tracking-[0.18em] uppercase text-ink-soft">
            Shift {today}
          </span>
        </div>
        <span className="font-mono text-[0.7rem] tracking-[0.18em] uppercase text-ink-mute flex items-center gap-2">
          <Activity className="h-3 w-3" strokeWidth={2} />
          Live
        </span>
      </div>

      {/* Title block */}
      <div className="mb-8">
        <p className="font-mono text-[0.65rem] tracking-[0.22em] uppercase text-gold font-semibold mb-3 flex items-center gap-2">
          <span className="inline-block h-[2px] w-6 bg-gold" />
          Operations Overview
        </p>
        <h1 className="font-display text-5xl sm:text-6xl font-bold tracking-tight uppercase leading-[1.0] text-ink">
          The Workbench.
        </h1>
        <p className="mt-3 text-ink-soft max-w-xl leading-relaxed">
          Stock on hand, money in motion, and what needs your attention this
          shift.
        </p>
      </div>

      {/* Metrics */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-8">
        <StatCard
          label="Revenue · MTD"
          value={fmt(stats?.revenueMTD || 0)}
          delta={{ value: "+12.4%", positive: true }}
          caption="vs last month"
          emphasis
        />
        <StatCard
          label="Expenses · MTD"
          value={fmt(stats?.expensesMTD || 0)}
          delta={{ value: "−4.1%", positive: true }}
          caption="vs last month"
        />
        <StatCard
          label="Outstanding"
          value={fmt(stats?.outstanding || 0)}
          delta={{ value: "+2.8%", positive: false }}
          caption="receivable"
        />
        <StatCard
          label="Low Stock"
          value={String(stats?.lowStockCount || 0)}
          caption="reorder needed"
        />
      </div>

      {/* Lower grid */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        <div className="lg:col-span-2 rounded-[var(--radius-lg)] border border-line bg-surface p-6">
          <div className="flex items-start justify-between mb-5">
            <div>
              <p className="font-mono text-[0.62rem] tracking-[0.22em] uppercase text-ink-mute font-semibold mb-1">
                Revenue
              </p>
              <h2 className="font-display text-xl font-semibold uppercase tracking-tight text-ink">
                Daily inflow
              </h2>
            </div>
            <div className="flex border border-line rounded-[var(--radius)] overflow-hidden">
              {["7D", "30D", "90D"].map((r, i) => (
                <button
                  key={r}
                  className={
                    "font-mono text-[0.7rem] tracking-wider px-3 py-1.5 border-r border-line last:border-0 " +
                    (i === 1
                      ? "bg-ink text-surface"
                      : "bg-surface text-ink-soft hover:bg-surface-2")
                  }
                >
                  {r}
                </button>
              ))}
            </div>
          </div>
          <ResponsiveContainer width="100%" height={260}>
            <AreaChart
              data={stats?.trendData || []}
              margin={{ top: 8, right: 0, left: -12, bottom: 0 }}
            >
              <defs>
                <linearGradient id="revFill" x1="0" y1="0" x2="0" y2="1">
                  <stop
                    offset="0%"
                    stopColor="var(--gold)"
                    stopOpacity={0.25}
                  />
                  <stop
                    offset="100%"
                    stopColor="var(--gold)"
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
                  borderRadius: "4px",
                  fontSize: "12px",
                  fontFamily: "var(--font-mono)",
                }}
                formatter={(value) => [
                  fmt(Number(value)),
                  "Revenue",
                ]}
              />
              <Area
                type="monotone"
                dataKey="revenue"
                stroke="var(--gold)"
                strokeWidth={2}
                fill="url(#revFill)"
                dot={false}
              />
            </AreaChart>
          </ResponsiveContainer>
        </div>

        <div className="rounded-[var(--radius-lg)] border border-line bg-surface p-6">
          <div className="flex items-center justify-between mb-4">
            <div>
              <p className="font-mono text-[0.62rem] tracking-[0.22em] uppercase text-ink-mute font-semibold mb-1">
                Last issued
              </p>
              <h2 className="font-display text-xl font-semibold uppercase tracking-tight text-ink">
                Invoices
              </h2>
            </div>
          </div>
          <div className="-mx-2">
            {stats?.recentInvoices?.map((inv: any, idx: number) => (
              <div
                key={inv.id}
                className={
                  "flex items-center justify-between px-2 py-3 " +
                  (idx > 0 ? "border-t border-line" : "")
                }
              >
                <div className="min-w-0">
                  <p className="font-mono text-sm text-ink truncate font-medium">
                    {inv.invoiceNo}
                  </p>
                  <p className="text-xs text-ink-mute truncate mt-0.5">
                    {inv.customer?.name}
                  </p>
                </div>
                <div className="text-right ml-3 flex-shrink-0">
                  <p className="font-mono font-semibold text-sm text-ink tabular">
                    {fmt(inv.total)}
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
              <p className="font-mono text-xs uppercase tracking-wider text-ink-mute text-center py-12">
                No invoices yet
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
