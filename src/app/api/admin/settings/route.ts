import { prisma } from "@/lib/prisma";
import { requireSuperAdmin, handleRouteError, errorResponse } from "@/lib/api";
import { PlatformSettingsSchema } from "@/lib/validations";
import { NextResponse } from "next/server";

const SETTINGS_ID = "platform";

async function getOrCreateSettings() {
  return prisma.platformSettings.upsert({
    where: { id: SETTINGS_ID },
    update: {},
    create: { id: SETTINGS_ID },
  });
}

export async function GET() {
  try {
    await requireSuperAdmin();
    const settings = await getOrCreateSettings();
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

    await getOrCreateSettings();
    const settings = await prisma.platformSettings.update({
      where: { id: SETTINGS_ID },
      data: parsed.data,
    });
    return NextResponse.json(settings);
  } catch (error) {
    return handleRouteError(error);
  }
}
