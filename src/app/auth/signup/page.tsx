"use client";

import { useState } from "react";
import { signIn } from "next-auth/react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { usePlatform } from "@/lib/hooks";

export default function SignupPage() {
  const router = useRouter();
  const platform = usePlatform();
  const platformName = platform?.platformName ?? "Simplex";
  const signupsDisabled = platform?.allowSignups === false;
  const [formData, setFormData] = useState({
    name: "",
    email: "",
    password: "",
    orgName: "",
    orgSlug: "",
  });
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const { name, value } = e.target;
    setFormData((prev) => ({
      ...prev,
      [name]: value,
      ...(name === "orgName"
        ? {
            orgSlug: value
              .toLowerCase()
              .replace(/[^a-z0-9]/g, "-")
              .replace(/-+/g, "-")
              .replace(/^-|-$/g, ""),
          }
        : {}),
    }));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setLoading(true);

    try {
      const res = await fetch("/api/auth/signup", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(formData),
      });

      const data = await res.json();

      if (!res.ok) {
        setError(data.error || "Signup failed");
        setLoading(false);
        return;
      }

      const result = await signIn("credentials", {
        email: formData.email,
        password: formData.password,
        redirect: false,
      });

      if (result?.ok) {
        router.push(`/${formData.orgSlug}/dashboard`);
      }
    } catch {
      setError("Something went wrong");
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen grid lg:grid-cols-2 bg-bg">
      {/* Left — industrial spec sheet */}
      <div className="hidden lg:flex relative overflow-hidden brushed flex-col justify-between p-12 m-4 rounded-[var(--radius-lg)]">
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
            New shop setup
          </p>
          <h2 className="font-display text-5xl sm:text-6xl font-bold uppercase leading-[1.0] tracking-tight text-surface">
            Open the
            <br />
            shop.
          </h2>
          <p className="mt-6 text-surface/70 leading-relaxed max-w-md">
            Set up your workspace in a few minutes — products, warehouses,
            and the counter all wired up for you.
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
            Set up
          </p>
          <h1 className="font-display text-4xl font-bold uppercase tracking-tight text-ink leading-[1.05]">
            New shop.
          </h1>
          <p className="mt-3 text-sm text-ink-soft">
            A few details and you&apos;re open for business.
          </p>

          {signupsDisabled ? (
            <div className="mt-8 space-y-4">
              <div className="rounded-[var(--radius)] border-l-4 border-gold bg-gold-soft px-4 py-3 text-sm text-ink">
                Public sign-ups are currently closed
                {platform?.supportEmail ? (
                  <>
                    {" "}— contact{" "}
                    <a href={`mailto:${platform.supportEmail}`} className="font-semibold underline underline-offset-4">
                      {platform.supportEmail}
                    </a>{" "}
                    to get set up.
                  </>
                ) : (
                  " — contact platform support to get set up."
                )}
              </div>
              <p className="text-center text-sm text-ink-soft pt-2 font-mono uppercase tracking-wider text-[0.7rem]">
                Already set up?{" "}
                <Link href="/auth/login" className="text-gold font-semibold hover:underline underline-offset-4">
                  Sign in
                </Link>
              </p>
            </div>
          ) : (
          <form onSubmit={handleSubmit} className="mt-8 space-y-5">
            {error && (
              <div className="rounded-[var(--radius)] border-l-4 border-danger bg-danger-soft px-4 py-3 text-sm text-danger font-mono">
                {error}
              </div>
            )}
            <div className="space-y-2">
              <Label htmlFor="name">Your Name</Label>
              <Input
                id="name"
                name="name"
                placeholder="Bruce Higiro"
                value={formData.name}
                onChange={handleChange}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                name="email"
                type="email"
                placeholder="you@hardware.shop"
                value={formData.email}
                onChange={handleChange}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="password">Password</Label>
              <Input
                id="password"
                name="password"
                type="password"
                placeholder="Min 6 characters"
                value={formData.password}
                onChange={handleChange}
                minLength={6}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="orgName">Shop Name</Label>
              <Input
                id="orgName"
                name="orgName"
                placeholder="Higiro Hardware Co."
                value={formData.orgName}
                onChange={handleChange}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="orgSlug">Workspace URL</Label>
              <div className="flex items-center gap-0 rounded-[var(--radius)] border border-line bg-surface focus-within:border-gold focus-within:ring-2 focus-within:ring-gold/30 transition-all">
                <span className="pl-3 pr-2 text-sm text-ink-mute select-none font-mono">
                  app /
                </span>
                <input
                  id="orgSlug"
                  name="orgSlug"
                  placeholder="higiro-hardware"
                  value={formData.orgSlug}
                  onChange={handleChange}
                  required
                  className="h-10 flex-1 bg-transparent pr-3 text-sm text-ink placeholder:text-ink-mute focus:outline-none font-mono"
                />
              </div>
            </div>

            <Button
              type="submit"
              size="lg"
              className="w-full"
              disabled={loading}
            >
              {loading ? "Setting up…" : "Open shop"}
            </Button>

            <p className="text-center text-sm text-ink-soft pt-2 font-mono uppercase tracking-wider text-[0.7rem]">
              Already set up?{" "}
              <Link
                href="/auth/login"
                className="text-gold font-semibold hover:underline underline-offset-4"
              >
                Sign in
              </Link>
            </p>
          </form>
          )}
        </div>
      </div>
    </div>
  );
}
