import "next-auth";

declare module "next-auth" {
  interface Session {
    user: {
      id: string;
      email: string;
      name?: string | null;
      orgId: string;
      orgSlug: string;
      role: string;
    };
  }
}

declare module "next-auth/jwt" {
  interface JWT {
    orgId: string;
    orgSlug: string;
    role: string;
  }
}
