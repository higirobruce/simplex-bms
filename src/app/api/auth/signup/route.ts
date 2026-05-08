import { prisma } from "@/lib/prisma";
import { NextResponse } from "next/server";
import bcrypt from "bcryptjs";

export async function POST(req: Request) {
  try {
    const { email, password, name, orgName, orgSlug } = await req.json();

    if (!email || !password || !orgName || !orgSlug) {
      return NextResponse.json({ error: "Missing required fields" }, { status: 400 });
    }

    // Check if slug already exists
    const existingOrg = await prisma.org.findUnique({ where: { slug: orgSlug } });
    if (existingOrg) {
      return NextResponse.json({ error: "Organization slug already taken" }, { status: 409 });
    }

    const hashedPassword = await bcrypt.hash(password, 10);

    const org = await prisma.org.create({
      data: {
        slug: orgSlug,
        name: orgName,
        users: {
          create: {
            email,
            password: hashedPassword,
            name: name || email.split("@")[0],
            role: "ADMIN",
          },
        },
        locations: {
          create: {
            name: "Main Warehouse",
          },
        },
      },
      include: { users: true },
    });

    return NextResponse.json({ orgSlug: org.slug, userId: org.users[0].id });
  } catch (error: any) {
    return NextResponse.json({ error: error.message || "Signup failed" }, { status: 500 });
  }
}
