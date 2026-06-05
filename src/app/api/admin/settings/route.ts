import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError, errorResponse } from "@/lib/api";
import { PlatformSettingsSchema } from "@/lib/validations";
import { getPlatformSettings, PLATFORM_SETTINGS_ID } from "@/lib/platform";
import { NextResponse } from "next/server";

export async function GET() {
  try {
    await requireSuperAdmin();
    const settings = await getPlatformSettings();
    return NextResponse.json(settings);
  } catch (error) {
    return handleRouteError(error);
  }
}

export async function PATCH(req: Request) {
  try {
    await requireSuperAdmin();
    const body = await req.json();
    const parsed = PlatformSettingsSchema.safeParse(body);
    if (!parsed.success) return errorResponse(parsed.error.issues[0].message, 422);

    await getPlatformSettings();
    const settings = await prisma.platformSettings.update({
      where: { id: PLATFORM_SETTINGS_ID },
      data: parsed.data,
    });
    return NextResponse.json(settings);
  } catch (error) {
    return handleRouteError(error);
  }
}
