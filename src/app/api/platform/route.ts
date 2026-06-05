import { getPlatformSettings, publicPlatformSettings } from "@/lib/platform";
import { NextResponse } from "next/server";

// Public, unauthenticated read of the non-sensitive platform settings used for
// branding, signup gating and shop defaults across the whole app.
export async function GET() {
  const settings = await getPlatformSettings();
  return NextResponse.json(publicPlatformSettings(settings));
}
