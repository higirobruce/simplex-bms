import { NextAuthOptions } from "next-auth";
import CredentialsProvider from "next-auth/providers/credentials";
import { prisma } from "@/lib/prisma";
import bcrypt from "bcryptjs";

export const authOptions: NextAuthOptions = {
  session: { strategy: "jwt" },
  pages: {
    signIn: "/auth/login",
  },
  providers: [
    CredentialsProvider({
      name: "credentials",
      credentials: {
        email: { label: "Email", type: "email" },
        password: { label: "Password", type: "password" },
      },
      async authorize(credentials) {
        if (!credentials?.email || !credentials?.password) return null;

        const user = await prisma.user.findFirst({
          where: { email: credentials.email },
          include: { org: true },
        });

        if (!user) return null;

        const isValid = await bcrypt.compare(credentials.password, user.password);
        if (!isValid) return null;

        // Block access to suspended shops (super admins have no org and are exempt)
        if (!user.isSuperAdmin && user.org?.status === "SUSPENDED") {
          throw new Error("This workspace has been suspended. Contact platform support.");
        }

        return {
          id: user.id,
          email: user.email,
          name: user.name,
          orgId: user.orgId ?? null,
          orgSlug: user.org?.slug ?? null,
          role: user.role,
          isSuperAdmin: user.isSuperAdmin,
        };
      },
    }),
  ],
  callbacks: {
    async jwt({ token, user }) {
      if (user) {
        token.orgId = (user as any).orgId;
        token.orgSlug = (user as any).orgSlug;
        token.role = (user as any).role;
        token.isSuperAdmin = (user as any).isSuperAdmin;
      }
      return token;
    },
    async session({ session, token }) {
      if (session.user) {
        (session.user as any).id = token.sub;
        (session.user as any).orgId = token.orgId;
        (session.user as any).orgSlug = token.orgSlug;
        (session.user as any).role = token.role;
        (session.user as any).isSuperAdmin = token.isSuperAdmin;
      }
      return session;
    },
  },
};
