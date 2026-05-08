"use client";

import { useState } from "react";
import { signIn } from "next-auth/react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export default function SignupPage() {
  const router = useRouter();
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
      {/* Left — editorial */}
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
            Begin your <span className="text-gold-soft">house.</span>
          </p>
          <p className="mt-6 text-surface/70 leading-relaxed">
            Set up your atelier in moments. Inventory, invoicing and the quiet
            instruments of fine business — all composed for you.
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
            Create account
          </p>
          <h1 className="font-serif text-4xl text-ink leading-tight">
            Establish your <span className="italic">atelier.</span>
          </h1>
          <p className="mt-3 text-sm text-ink-soft">
            Set up your workspace in a few quiet steps.
          </p>

          <form onSubmit={handleSubmit} className="mt-8 space-y-5">
            {error && (
              <div className="rounded-[var(--radius)] border border-danger/20 bg-danger-soft px-4 py-3 text-sm text-danger">
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
                placeholder="you@company.com"
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
                placeholder="Minimum 6 characters"
                value={formData.password}
                onChange={handleChange}
                minLength={6}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="orgName">Business Name</Label>
              <Input
                id="orgName"
                name="orgName"
                placeholder="Maison Higiro"
                value={formData.orgName}
                onChange={handleChange}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="orgSlug">Workspace URL</Label>
              <div className="flex items-center gap-0 rounded-[var(--radius)] border border-line bg-surface-2/60 focus-within:bg-surface focus-within:border-accent/40 focus-within:ring-4 focus-within:ring-accent/10 transition-all">
                <span className="pl-4 pr-2 text-sm text-ink-mute select-none">
                  app /
                </span>
                <input
                  id="orgSlug"
                  name="orgSlug"
                  placeholder="maison-higiro"
                  value={formData.orgSlug}
                  onChange={handleChange}
                  required
                  className="h-11 flex-1 bg-transparent pr-4 text-sm text-ink placeholder:text-ink-mute focus:outline-none"
                />
              </div>
            </div>

            <Button
              type="submit"
              size="lg"
              className="w-full"
              disabled={loading}
            >
              {loading ? "Establishing…" : "Create account"}
            </Button>

            <p className="text-center text-sm text-ink-soft pt-2">
              Already have an account?{" "}
              <Link
                href="/auth/login"
                className="text-accent font-medium hover:underline underline-offset-4"
              >
                Sign in
              </Link>
            </p>
          </form>
        </div>
      </div>
    </div>
  );
}
