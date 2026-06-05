"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/page-header";
import { apiCall } from "@/lib/fetcher";
import { usePlatform } from "@/lib/hooks";
import { Store, CheckCircle2, Ban, Users } from "lucide-react";

function Stat({ label, value, icon: Icon }: { label: string; value: number | string; icon: any }) {
  return (
    <Card>
      <CardContent className="flex items-center gap-4 py-5">
        <span className="flex h-11 w-11 items-center justify-center rounded-full bg-accent-soft">
          <Icon className="h-5 w-5 text-accent" strokeWidth={1.75} />
        </span>
        <div>
          <p className="font-mono text-[0.62rem] tracking-[0.18em] uppercase text-ink-mute">{label}</p>
          <p className="font-display text-3xl font-bold text-ink leading-none mt-1">{value}</p>
        </div>
      </CardContent>
    </Card>
  );
}

export default function AdminOverviewPage() {
  const platform = usePlatform();
  const { data, isLoading } = useQuery({
    queryKey: ["admin-stats"],
    queryFn: () => apiCall("/api/admin/stats"),
  });

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Platform"
        title="Console"
        description={`Every shop on ${platform?.platformName ?? "Simplex"} at a glance — provision, oversee, and step in when needed.`}
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Stat label="Total shops" value={isLoading ? "—" : data?.totalShops ?? 0} icon={Store} />
        <Stat label="Active" value={isLoading ? "—" : data?.activeShops ?? 0} icon={CheckCircle2} />
        <Stat label="Suspended" value={isLoading ? "—" : data?.suspendedShops ?? 0} icon={Ban} />
        <Stat label="Shop users" value={isLoading ? "—" : data?.totalUsers ?? 0} icon={Users} />
      </div>

      <div className="mt-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="font-display text-xl font-bold uppercase tracking-tight text-ink">Newest shops</h2>
          <Link href="/admin/shops" className="text-sm text-gold hover:underline underline-offset-4">
            View all
          </Link>
        </div>
        <Card>
          <CardContent className="py-2">
            {isLoading ? (
              <div className="py-8 text-center font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">
                Loading…
              </div>
            ) : (data?.recentShops?.length ?? 0) === 0 ? (
              <div className="py-8 text-center text-sm text-ink-mute">No shops yet.</div>
            ) : (
              <ul className="divide-y divide-line/70">
                {data.recentShops.map((s: any) => (
                  <li key={s.id} className="flex items-center justify-between py-3.5">
                    <div className="min-w-0">
                      <Link href="/admin/shops" className="font-medium text-ink hover:text-gold">
                        {s.name}
                      </Link>
                      <p className="font-mono text-[0.65rem] tracking-[0.1em] uppercase text-ink-mute mt-0.5">
                        /{s.slug} · {s._count.users} user{s._count.users === 1 ? "" : "s"}
                      </p>
                    </div>
                    <Badge variant={s.status === "ACTIVE" ? "success" : "destructive"}>
                      {s.status.toLowerCase()}
                    </Badge>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
