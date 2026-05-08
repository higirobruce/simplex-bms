"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  LayoutDashboard,
  Package,
  Users,
  FileText,
  Truck,
  Receipt,
  Settings,
  LogOut,
  Menu,
  X,
  Warehouse,
  ShoppingCart,
  PackageOpen,
} from "lucide-react";
import { signOut } from "next-auth/react";
import { cn } from "@/lib/utils";
import { useState } from "react";

const navGroups: {
  label: string;
  items: { href: string; label: string; icon: any }[];
}[] = [
  {
    label: "Workspace",
    items: [
      { href: "dashboard", label: "Overview", icon: LayoutDashboard },
      { href: "products", label: "Products", icon: Package },
      { href: "stock", label: "Inventory", icon: Warehouse },
    ],
  },
  {
    label: "Operations",
    items: [
      { href: "sales", label: "Sales", icon: ShoppingCart },
      { href: "procurement", label: "Procurement", icon: PackageOpen },
    ],
  },
  {
    label: "Commerce",
    items: [
      { href: "invoices", label: "Invoices", icon: FileText },
      { href: "expenses", label: "Expenses", icon: Receipt },
    ],
  },
  {
    label: "Relationships",
    items: [
      { href: "customers", label: "Customers", icon: Users },
      { href: "vendors", label: "Vendors", icon: Truck },
    ],
  },
];

export function SideNav({ slug }: { slug: string }) {
  const pathname = usePathname();
  const [mobileOpen, setMobileOpen] = useState(false);

  return (
    <>
      {/* Mobile toggle */}
      <button
        className="fixed top-4 left-4 z-50 rounded-full bg-surface p-2.5 shadow-panel border border-line lg:hidden"
        onClick={() => setMobileOpen(!mobileOpen)}
        aria-label="Toggle navigation"
      >
        {mobileOpen ? <X size={18} /> : <Menu size={18} />}
      </button>

      {/* Overlay */}
      {mobileOpen && (
        <div
          className="fixed inset-0 z-30 bg-ink/30 backdrop-blur-sm lg:hidden"
          onClick={() => setMobileOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-40 w-72 transform bg-surface transition-transform lg:relative lg:translate-x-0",
          "lg:my-0 lg:rounded-[var(--radius-lg)] lg:shadow-panel lg:border lg:border-line",
          mobileOpen ? "translate-x-0" : "-translate-x-full"
        )}
      >
        <div className="flex h-full flex-col">
          {/* Brand */}
          <div className="flex items-center gap-3 px-7 pt-8 pb-6">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-accent text-surface shadow-sm">
              <span className="font-serif italic text-xl leading-none -mt-0.5">
                S
              </span>
            </div>
            <div className="flex flex-col leading-tight">
              <span className="font-serif italic text-[1.35rem] text-ink leading-none">
                Simplex
              </span>
              <span className="text-[0.65rem] tracking-[0.2em] uppercase text-ink-mute mt-1">
                Atelier · {slug}
              </span>
            </div>
          </div>

          <div className="mx-7 h-px bg-line" />

          {/* Nav */}
          <div className="flex-1 overflow-y-auto px-4 py-6">
            <ul className="space-y-7">
              {navGroups.map((group) => (
                <li key={group.label}>
                  <p className="px-4 mb-2 text-[0.6rem] tracking-[0.22em] uppercase text-ink-mute font-medium">
                    {group.label}
                  </p>
                  <ul className="space-y-0.5">
                    {group.items.map((item) => {
                      const fullHref = `/${slug}/${item.href}`;
                      const isActive = pathname?.startsWith(fullHref);
                      return (
                        <li key={item.href}>
                          <Link
                            href={fullHref}
                            onClick={() => setMobileOpen(false)}
                            className={cn(
                              "group relative flex items-center gap-3 rounded-xl px-4 py-2.5 text-sm transition-all",
                              isActive
                                ? "bg-accent-soft text-accent font-medium"
                                : "text-ink-soft hover:bg-surface-2 hover:text-ink"
                            )}
                          >
                            {isActive && (
                              <span className="absolute left-1.5 top-1/2 -translate-y-1/2 h-5 w-[2px] rounded-full bg-accent" />
                            )}
                            <item.icon
                              size={16}
                              strokeWidth={1.75}
                              className={cn(
                                "transition-colors",
                                isActive ? "text-accent" : "text-ink-mute group-hover:text-ink"
                              )}
                            />
                            <span>{item.label}</span>
                          </Link>
                        </li>
                      );
                    })}
                  </ul>
                </li>
              ))}
            </ul>
          </div>

          {/* Footer */}
          <div className="border-t border-line px-4 py-4 space-y-0.5">
            <Link
              href={`/${slug}/settings`}
              className={cn(
                "flex items-center gap-3 rounded-xl px-4 py-2.5 text-sm transition-colors",
                pathname?.startsWith(`/${slug}/settings`)
                  ? "bg-accent-soft text-accent font-medium"
                  : "text-ink-soft hover:bg-surface-2 hover:text-ink"
              )}
            >
              <Settings size={16} strokeWidth={1.75} />
              Settings
            </Link>
            <button
              onClick={() => signOut({ callbackUrl: "/auth/login" })}
              className="flex w-full items-center gap-3 rounded-xl px-4 py-2.5 text-sm text-ink-soft hover:bg-surface-2 hover:text-ink transition-colors"
            >
              <LogOut size={16} strokeWidth={1.75} />
              Sign out
            </button>
          </div>
        </div>
      </aside>
    </>
  );
}
