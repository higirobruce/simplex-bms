"use client";

import { useSession } from "next-auth/react";
import { useRouter, useParams } from "next/navigation";
import { useState } from "react";
import { ShieldAlert, LogOut } from "lucide-react";

export function ImpersonationBanner() {
  const { data: session } = useSession();
  const router = useRouter();
  const params = useParams();
  const [exiting, setExiting] = useState(false);

  if (!(session?.user as any)?.isSuperAdmin) return null;

  const slug = params?.slug as string;

  async function exit() {
    setExiting(true);
    try {
      await fetch("/api/admin/impersonate", { method: "DELETE" });
      router.push("/admin");
      router.refresh();
    } finally {
      setExiting(false);
    }
  }

  return (
    <div className="flex items-center justify-between gap-3 bg-gold px-5 py-2.5 text-surface">
      <div className="flex items-center gap-2.5 min-w-0">
        <ShieldAlert size={16} strokeWidth={2} className="shrink-0" />
        <p className="truncate font-mono text-[0.7rem] tracking-[0.12em] uppercase">
          Platform mode — acting inside <span className="font-bold">{slug}</span>
        </p>
      </div>
      <button
        onClick={exit}
        disabled={exiting}
        className="flex items-center gap-1.5 rounded-[var(--radius)] border border-surface/40 px-3 py-1 text-[0.7rem] font-semibold uppercase tracking-wide transition-colors hover:bg-surface/15 disabled:opacity-50"
      >
        <LogOut size={13} strokeWidth={2} />
        {exiting ? "Exiting…" : "Exit to console"}
      </button>
    </div>
  );
}
