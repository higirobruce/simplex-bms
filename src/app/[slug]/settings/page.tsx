"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { useTenant } from "@/lib/hooks";
import { PageHeader } from "@/components/page-header";
import { Building2, User, Shield } from "lucide-react";

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between py-3 border-b border-line/70 last:border-0">
      <dt className="text-[0.7rem] tracking-[0.18em] uppercase text-ink-mute font-medium">
        {label}
      </dt>
      <dd className="text-sm text-ink">{value}</dd>
    </div>
  );
}

export default function SettingsPage() {
  const { slug, role, user } = useTenant();

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Workspace"
        title="Settings"
        description="The particulars of your atelier — organisation, account, and access."
      />

      <div className="grid gap-5 max-w-3xl">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-3 text-xl">
              <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
                <Building2 className="h-4 w-4 text-accent" strokeWidth={1.75} />
              </span>
              Organisation
            </CardTitle>
          </CardHeader>
          <CardContent>
            <dl>
              <Row label="Slug" value={slug} />
              <Row label="Currency" value="RWF" />
              <Row label="Timezone" value="Africa / Kigali" />
            </dl>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-3 text-xl">
              <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
                <User className="h-4 w-4 text-accent" strokeWidth={1.75} />
              </span>
              Account
            </CardTitle>
          </CardHeader>
          <CardContent>
            <dl>
              <Row label="Email" value={user?.email || "—"} />
              <Row label="Name" value={user?.name || "—"} />
            </dl>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-3 text-xl">
              <span className="flex h-9 w-9 items-center justify-center rounded-full bg-accent-soft">
                <Shield className="h-4 w-4 text-accent" strokeWidth={1.75} />
              </span>
              Role &amp; Permissions
            </CardTitle>
          </CardHeader>
          <CardContent>
            <dl>
              <Row
                label="Role"
                value={
                  role ? (
                    <Badge variant="default">{role.toLowerCase()}</Badge>
                  ) : (
                    "—"
                  )
                }
              />
            </dl>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
