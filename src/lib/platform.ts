import { prisma } from "@/lib/prisma";

export const PLATFORM_SETTINGS_ID = "platform";

// The platform settings are a single row (id = "platform"). Read-only on the
// hot path; the row is created lazily only the first time it's missing so we
// don't issue a write on every page render.
export async function getPlatformSettings() {
  const existing = await prisma.platformSettings.findUnique({
    where: { id: PLATFORM_SETTINGS_ID },
  });
  if (existing) return existing;
  try {
    return await prisma.platformSettings.create({ data: { id: PLATFORM_SETTINGS_ID } });
  } catch (err: any) {
    // Lost a race to create the singleton — re-read the row the winner inserted.
    if (err?.code === "P2002") {
      return prisma.platformSettings.findUniqueOrThrow({ where: { id: PLATFORM_SETTINGS_ID } });
    }
    throw err;
  }
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
