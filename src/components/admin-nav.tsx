"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { LayoutDashboard, Store, Users, SlidersHorizontal, LogOut } from "lucide-react";
import { signOut } from "next-auth/react";
import { cn } from "@/lib/utils";

const items: { href: string; label: string; icon: any }[] = [
  { href: "/admin", label: "Overview", icon: LayoutDashboard },
  { href: "/admin/shops", label: "Shops", icon: Store },
  { href: "/admin/users", label: "Users", icon: Users },
  { href: "/admin/settings", label: "Platform", icon: SlidersHorizontal },
];

export function AdminNav() {
  const pathname = usePathname();

  return (
    <aside className="fixed inset-y-0 left-0 z-40 w-72 bg-surface lg:relative lg:my-0 lg:rounded-[var(--radius-lg)] lg:border lg:border-line">
      <div className="flex h-full flex-col">
        {/* Brand */}
        <div className="flex items-center gap-3 px-6 pt-7 pb-5">
          <div className="stencil-block h-10 w-10 text-lg">◼</div>
          <div className="flex flex-col leading-tight">
            <span className="font-display font-bold text-[1.05rem] text-ink leading-none tracking-tight uppercase">
              Simplex
            </span>
            <span className="font-mono text-[0.62rem] tracking-[0.16em] uppercase text-gold mt-1.5">
              Platform Console
            </span>
          </div>
        </div>

        <div className="mx-6 h-px bg-line" />

        {/* Nav */}
        <div className="flex-1 overflow-y-auto px-3 py-5">
          <p className="px-4 mb-2 font-mono text-[0.6rem] tracking-[0.18em] uppercase text-ink-mute font-medium">
            / Administration
          </p>
          <ul className="space-y-0.5">
            {items.map((item) => {
              const isActive =
                item.href === "/admin"
                  ? pathname === "/admin"
                  : pathname?.startsWith(item.href);
              return (
                <li key={item.href}>
                  <Link
                    href={item.href}
                    className={cn(
                      "group relative flex items-center gap-3 rounded-[var(--radius)] px-4 py-2 text-sm transition-all",
                      isActive
                        ? "bg-ink text-surface font-medium"
                        : "text-ink-soft hover:bg-surface-2 hover:text-ink"
                    )}
                  >
                    {isActive && (
                      <span className="absolute left-0 top-1/2 -translate-y-1/2 h-5 w-[3px] bg-gold" />
                    )}
                    <item.icon
                      size={15}
                      strokeWidth={2}
                      className={cn(
                        "transition-colors shrink-0",
                        isActive ? "text-gold" : "text-ink-mute group-hover:text-ink"
                      )}
                    />
                    <span className="tracking-tight">{item.label}</span>
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>

        {/* Footer */}
        <div className="border-t border-line px-3 py-3">
          <button
            onClick={() => signOut({ callbackUrl: "/auth/login" })}
            className="flex w-full items-center gap-3 rounded-[var(--radius)] px-4 py-2 text-sm text-ink-soft hover:bg-surface-2 hover:text-ink transition-colors"
          >
            <LogOut size={15} strokeWidth={2} />
            Sign out
          </button>
        </div>
      </div>
    </aside>
  );
}
