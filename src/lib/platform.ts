import { prisma } from "@/lib/prisma";

export const PLATFORM_SETTINGS_ID = "platform";

// The platform settings are a single row (id = "platform"). Read-or-create so
// callers never have to deal with a missing row.
export async function getPlatformSettings() {
  return prisma.platformSettings.upsert({
    where: { id: PLATFORM_SETTINGS_ID },
    update: {},
    create: { id: PLATFORM_SETTINGS_ID },
  });
}

// Fields safe to expose publicly (no secrets) — used for branding, signup
// gating and shop defaults across the app.
export function publicPlatformSettings(s: {
  platformName: string;
  supportEmail: string | null;
  defaultCurrency: string;
  defaultTimezone: string;
  allowSignups: boolean;
}) {
  return {
    platformName: s.platformName,
    supportEmail: s.supportEmail,
    defaultCurrency: s.defaultCurrency,
    defaultTimezone: s.defaultTimezone,
    allowSignups: s.allowSignups,
  };
}
