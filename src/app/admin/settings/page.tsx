"use client";

import { useEffect, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useToast } from "@/components/ui/toast";
import { PageHeader } from "@/components/page-header";
import { apiCall } from "@/lib/fetcher";
import { SlidersHorizontal, UserPlus } from "lucide-react";

export default function PlatformSettingsPage() {
  const queryClient = useQueryClient();
  const toast = useToast();
  const [form, setForm] = useState({
    platformName: "",
    supportEmail: "",
    defaultCurrency: "",
    defaultTimezone: "",
    allowSignups: true,
  });

  const { data, isLoading } = useQuery({
    queryKey: ["platform-settings"],
    queryFn: () => apiCall("/api/admin/settings"),
  });

  useEffect(() => {
    if (data) {
      setForm({
        platformName: data.platformName ?? "",
        supportEmail: data.supportEmail ?? "",
        defaultCurrency: data.defaultCurrency ?? "RWF",
        defaultTimezone: data.defaultTimezone ?? "Africa/Kigali",
        allowSignups: data.allowSignups ?? true,
      });
    }
  }, [data]);

  const save = useMutation({
    mutationFn: (body: any) => apiCall("/api/admin/settings", { method: "PATCH", body: JSON.stringify(body) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["platform-settings"] });
      toast.success("Platform settings saved");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Platform"
        title="Settings"
        description="Global defaults and controls that apply across every shop."
      />

      <form
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({
            platformName: form.platformName,
            supportEmail: form.supportEmail.trim() || null,
            defaultCurrency: form.defaultCurrency,
            defaultTimezone: form.defaultTimezone,
            allowSignups: form.allowSignups,
          });
        }}
        className="grid gap-5 max-w-2xl"
      >
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-3 text-xl">
              <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
                <SlidersHorizontal className="h-4 w-4 text-accent" strokeWidth={1.75} />
              </span>
              General
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label>Platform name</Label>
              <Input
                value={form.platformName}
                onChange={(e) => setForm({ ...form, platformName: e.target.value })}
                disabled={isLoading}
                required
              />
            </div>
            <div className="space-y-2">
              <Label>Support email</Label>
              <Input
                type="email"
                placeholder="support@example.com"
                value={form.supportEmail}
                onChange={(e) => setForm({ ...form, supportEmail: e.target.value })}
                disabled={isLoading}
              />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>Default currency</Label>
                <Input
                  value={form.defaultCurrency}
                  onChange={(e) => setForm({ ...form, defaultCurrency: e.target.value.toUpperCase() })}
                  maxLength={8}
                  disabled={isLoading}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label>Default timezone</Label>
                <Input
                  value={form.defaultTimezone}
                  onChange={(e) => setForm({ ...form, defaultTimezone: e.target.value })}
                  disabled={isLoading}
                  required
                />
              </div>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-3 text-xl">
              <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
                <UserPlus className="h-4 w-4 text-accent" strokeWidth={1.75} />
              </span>
              Sign-ups
            </CardTitle>
          </CardHeader>
          <CardContent>
            <label className="flex items-center justify-between gap-4 cursor-pointer">
              <div>
                <p className="text-sm font-medium text-ink">Allow public sign-ups</p>
                <p className="text-sm text-ink-soft mt-0.5">
                  When off, new shops can only be created from this console.
                </p>
              </div>
              <input
                type="checkbox"
                checked={form.allowSignups}
                onChange={(e) => setForm({ ...form, allowSignups: e.target.checked })}
                disabled={isLoading}
                className="h-5 w-5 accent-[var(--gold)] cursor-pointer"
              />
            </label>
          </CardContent>
        </Card>

        <div className="flex justify-end">
          <Button type="submit" disabled={save.isPending || isLoading}>
            {save.isPending ? "Saving…" : "Save settings"}
          </Button>
        </div>
      </form>
    </div>
  );
}
