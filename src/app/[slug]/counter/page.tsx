"use client";

import { Suspense } from "react";
import { useQuery } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { PageHeader } from "@/components/page-header";
import { CounterView } from "@/components/counter-view";
import { useTenant } from "@/lib/hooks";
import { shiftDay, type CounterResponse } from "@/lib/counter-view";

function CounterContent() {
  const { slug } = useTenant();
  const day = useSearchParams().get("day");

  const { data, isLoading, isError } = useQuery<CounterResponse>({
    queryKey: ["counter", slug, day],
    queryFn: async () => {
      const r = await fetch(`/api/${slug}/counter${day ? `?day=${encodeURIComponent(day)}` : ""}`);
      if (!r.ok) throw new Error("Could not load the counter");
      return r.json();
    },
    enabled: !!slug,
    refetchInterval: 45_000,
    refetchOnWindowFocus: true,
  });

  const nav = "rounded-full border border-line bg-surface px-3 py-1.5 text-sm text-ink-soft hover:bg-surface-3";
  return (
    <>
      <PageHeader
        eyebrow="Counter"
        title="The counter"
        description="Each shift's drawer against what the till says it took. Read-only."
        actions={
          data ? (
            <div className="flex gap-2">
              <Link className={nav} href={`?day=${shiftDay(data.day, -1)}`}>← Previous day</Link>
              <Link className={nav} href={`?day=${shiftDay(data.day, 1)}`}>Next day →</Link>
            </div>
          ) : undefined
        }
      />
      {isLoading && <p className="text-sm text-ink-soft">Loading shifts…</p>}
      {isError && <p role="alert" className="text-sm text-danger">Could not load the counter. It will retry shortly.</p>}
      {data && <CounterView data={data} tz={data.timezone} slug={slug} />}
    </>
  );
}

export default function CounterPage() {
  return (
    <Suspense fallback={null}>
      <CounterContent />
    </Suspense>
  );
}
