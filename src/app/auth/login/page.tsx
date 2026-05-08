"use client";

import { useState } from "react";
import { signIn } from "next-auth/react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export default function LoginPage() {
  const router = useRouter();
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
      {/* Left — editorial panel */}
      <div className="hidden lg:flex relative overflow-hidden bg-accent text-surface flex-col justify-between p-14 m-5 rounded-[var(--radius-lg)]">
        <div
          className="absolute inset-0 opacity-30 pointer-events-none"
          style={{
            background:
              "radial-gradient(circle at 30% 20%, var(--gold) 0%, transparent 45%), radial-gradient(circle at 80% 80%, rgba(255,255,255,0.06) 0%, transparent 50%)",
          }}
        />
        <div className="relative">
          <div className="flex items-center gap-3">
            <div className="h-11 w-11 rounded-xl bg-surface/10 backdrop-blur flex items-center justify-center border border-surface/15">
              <span className="font-serif italic text-2xl leading-none -mt-0.5">
                S
              </span>
            </div>
            <div className="leading-tight">
              <p className="font-serif italic text-2xl">Simplex</p>
              <p className="text-[0.6rem] tracking-[0.22em] uppercase text-gold-soft mt-0.5">
                Atelier
              </p>
            </div>
          </div>
        </div>

        <div className="relative max-w-md">
          <p className="font-serif italic text-5xl leading-[1.1] text-surface">
            Business management,
            <br />
            <span className="text-gold-soft">refined.</span>
          </p>
          <p className="mt-6 text-surface/70 leading-relaxed">
            A considered system for inventory, invoicing and the quiet rituals
            of running a fine business. Composed with care.
          </p>
        </div>

        <div className="relative flex items-center gap-4 text-xs tracking-[0.18em] uppercase text-surface/50">
          <span>Est. 2025</span>
          <span className="h-px flex-1 bg-surface/15" />
          <span>Volume I</span>
        </div>
      </div>

      {/* Right — form */}
      <div className="flex items-center justify-center p-6 lg:p-12">
        <div className="w-full max-w-sm">
          <div className="lg:hidden flex items-center gap-3 mb-8">
            <div className="h-10 w-10 rounded-xl bg-accent text-surface flex items-center justify-center">
              <span className="font-serif italic text-xl leading-none -mt-0.5">
                S
              </span>
            </div>
            <p className="font-serif italic text-xl text-ink">Simplex</p>
          </div>

          <p className="text-[0.7rem] tracking-[0.22em] uppercase text-gold font-medium mb-3">
            Welcome back
          </p>
          <h1 className="font-serif text-4xl text-ink leading-tight">
            Sign in to your <span className="italic">atelier.</span>
          </h1>
          <p className="mt-3 text-sm text-ink-soft">
            Enter your credentials to access your workspace.
          </p>

          <form onSubmit={handleSubmit} className="mt-9 space-y-5">
            {error && (
              <div className="rounded-[var(--radius)] border border-danger/20 bg-danger-soft px-4 py-3 text-sm text-danger">
                {error}
              </div>
            )}
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                placeholder="you@company.com"
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
              {loading ? "Signing in…" : "Sign in"}
            </Button>

            <p className="text-center text-sm text-ink-soft pt-2">
              Don&apos;t have an account?{" "}
              <Link
                href="/auth/signup"
                className="text-accent font-medium hover:underline underline-offset-4"
              >
                Create one
              </Link>
            </p>
          </form>
        </div>
      </div>
    </div>
  );
}
