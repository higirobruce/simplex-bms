"use client";

import { useState } from "react";
import { signIn } from "next-auth/react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { usePlatform } from "@/lib/hooks";

export default function LoginPage() {
  const router = useRouter();
  const platform = usePlatform();
  const platformName = platform?.platformName ?? "Simplex";
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setLoading(true);

    const result = await signIn("credentials", {
      email,
      password,
      redirect: false,
    });

    if (result?.error) {
      setError("Invalid email or password");
      setLoading(false);
      return;
    }

    const res = await fetch("/api/auth/session");
    const session = await res.json();
    if (session?.user?.orgSlug) {
      router.push(`/${session.user.orgSlug}/dashboard`);
    } else {
      router.push("/");
    }
  };

  return (
    <div className="min-h-screen grid lg:grid-cols-2 bg-bg">
      {/* Left — industrial spec sheet */}
      <div className="hidden lg:flex relative overflow-hidden brushed flex-col justify-between p-12 m-4 rounded-[var(--radius-lg)]">
        {/* Grid texture overlay */}
        <div
          className="absolute inset-0 opacity-[0.06] pointer-events-none"
          style={{
            backgroundImage:
              "linear-gradient(var(--surface) 1px, transparent 1px), linear-gradient(90deg, var(--surface) 1px, transparent 1px)",
            backgroundSize: "32px 32px",
          }}
        />
        <div
          className="absolute top-0 left-0 right-0 h-1 pointer-events-none"
          style={{ background: "var(--gold)" }}
        />

        <div className="relative">
          <div className="flex items-center gap-3">
            <div className="stencil-block h-11 w-11 text-lg bg-surface/10 border border-surface/20">
              ◼
            </div>
            <div className="leading-tight">
              <p className="font-display font-bold text-xl text-surface uppercase tracking-tight">
                {platformName}
              </p>
              <p className="font-mono text-[0.6rem] tracking-[0.22em] uppercase text-gold mt-1">
                / Workbench
              </p>
            </div>
          </div>
        </div>

        <div className="relative max-w-lg">
          <p className="font-mono text-[0.7rem] tracking-[0.22em] uppercase text-gold font-semibold mb-4 flex items-center gap-2">
            <span className="inline-block h-[2px] w-8 bg-gold" />
            Hardware Operations
          </p>
          <h2 className="font-display text-5xl sm:text-6xl font-bold uppercase leading-[1.0] tracking-tight text-surface">
            Stock.
            <br />
            Sell.
            <br />
            Restock.
          </h2>
          <p className="mt-6 text-surface/70 leading-relaxed max-w-md">
            A no-nonsense system for hardware shops — inventory across
            warehouses, sales orders, goods receipts, and the daily counter
            work.
          </p>
        </div>

        <div className="relative grid grid-cols-3 gap-px bg-surface/10 font-mono text-[0.62rem] tracking-[0.18em] uppercase">
          <div className="bg-[#1A1B1E] px-3 py-2.5">
            <p className="text-surface/40">Build</p>
            <p className="text-surface mt-0.5">v1.0</p>
          </div>
          <div className="bg-[#1A1B1E] px-3 py-2.5">
            <p className="text-surface/40">Module</p>
            <p className="text-surface mt-0.5">HW-OPS</p>
          </div>
          <div className="bg-[#1A1B1E] px-3 py-2.5">
            <p className="text-surface/40">Status</p>
            <p className="text-success mt-0.5">● Online</p>
          </div>
        </div>
      </div>

      {/* Right — form */}
      <div className="flex items-center justify-center p-6 lg:p-12">
        <div className="w-full max-w-sm">
          <div className="lg:hidden flex items-center gap-3 mb-8">
            <div className="stencil-block h-10 w-10 text-base">◼</div>
            <p className="font-display font-bold text-xl text-ink uppercase tracking-tight">
              {platformName}
            </p>
          </div>

          <p className="font-mono text-[0.65rem] tracking-[0.22em] uppercase text-gold font-semibold mb-3 flex items-center gap-2">
            <span className="inline-block h-[2px] w-6 bg-gold" />
            Sign in
          </p>
          <h1 className="font-display text-4xl font-bold uppercase tracking-tight text-ink leading-[1.05]">
            Workbench access.
          </h1>
          <p className="mt-3 text-sm text-ink-soft">
            Enter your credentials to clock in.
          </p>

          <form onSubmit={handleSubmit} className="mt-8 space-y-5">
            {error && (
              <div className="rounded-[var(--radius)] border-l-4 border-danger bg-danger-soft px-4 py-3 text-sm text-danger font-mono">
                {error}
              </div>
            )}
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                placeholder="you@hardware.shop"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="password">Password</Label>
              <Input
                id="password"
                type="password"
                placeholder="••••••••"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
              />
            </div>

            <Button
              type="submit"
              size="lg"
              className="w-full"
              disabled={loading}
            >
              {loading ? "Authenticating…" : "Clock in"}
            </Button>

            {platform?.allowSignups !== false && (
              <p className="text-center text-sm text-ink-soft pt-2 font-mono uppercase tracking-wider text-[0.7rem]">
                Don&apos;t have an account?{" "}
                <Link
                  href="/auth/signup"
                  className="text-gold font-semibold hover:underline underline-offset-4"
                >
                  Set up shop
                </Link>
              </p>
            )}

            {platform?.supportEmail && (
              <p className="text-center text-[0.7rem] text-ink-mute pt-1 font-mono uppercase tracking-wider">
                Need help?{" "}
                <a
                  href={`mailto:${platform.supportEmail}`}
                  className="text-ink-soft hover:text-ink underline-offset-4 hover:underline"
                >
                  {platform.supportEmail}
                </a>
              </p>
            )}
          </form>
        </div>
      </div>
    </div>
  );
}
