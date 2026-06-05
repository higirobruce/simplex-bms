import { getServerSession } from "next-auth";
import { authOptions } from "@/lib/auth";
import { NextResponse } from "next/server";
import { cookies } from "next/headers";

// Cookie a super admin carries while "acting inside" a specific shop.
export const ACTING_ORG_COOKIE = "simplex_acting_org";

// Typed error classes for distinguishing auth vs server errors
export class AuthError extends Error {
  constructor(message = "Unauthorized") {
    super(message);
    this.name = "AuthError";
  }
}

export class ForbiddenError extends Error {
  constructor(message = "Forbidden") {
    super(message);
    this.name = "ForbiddenError";
  }
}

// Permissions
export type Permission =
  | "products:write"
  | "products:delete"
  | "customers:write"
  | "customers:delete"
  | "vendors:write"
  | "vendors:delete"
  | "invoices:write"
  | "invoices:void"
  | "expenses:write"
  | "expenses:delete"
  | "stock:adjust"
  | "payments:write"
  | "locations:write"
  | "locations:delete"
  | "sales:write"
  | "sales:approve"
  | "procurement:write"
  | "procurement:approve"
  | "org:write"
  | "users:write";

const ALL_PERMISSIONS: Permission[] = [
  "products:write", "products:delete",
  "customers:write", "customers:delete",
  "vendors:write", "vendors:delete",
  "invoices:write", "invoices:void",
  "expenses:write", "expenses:delete",
  "stock:adjust", "payments:write",
  "locations:write", "locations:delete",
  "sales:write", "sales:approve",
  "procurement:write", "procurement:approve",
  "org:write", "users:write",
];

const ROLE_PERMISSIONS: Record<string, Permission[]> = {
  ADMIN: ALL_PERMISSIONS,
  MANAGER: [
    "products:write", "products:delete",
    "customers:write", "customers:delete",
    "vendors:write", "vendors:delete",
    "invoices:write",
    "expenses:write", "expenses:delete",
    "stock:adjust",
    "payments:write",
    "locations:write", "locations:delete",
    "sales:write", "sales:approve",
    "procurement:write", "procurement:approve",
  ],
  ACCOUNTANT: [
    "invoices:write",
    "expenses:write",
    "payments:write",
    "customers:write",
    "sales:write",
    "procurement:write",
  ],
  VIEWER: [],
};

interface TenantContext {
  orgId: string;
  userId: string;
  role: string;
}

export async function getTenantContext(): Promise<TenantContext> {
  const session = await getServerSession(authOptions);
  if (!session?.user) throw new AuthError();
  const user = session.user as any;

  // Super admins have no org of their own. They operate on a shop only while
  // "acting inside" it (impersonation), identified by the acting-org cookie.
  // Inside that shop they hold full ADMIN permissions.
  if (user.isSuperAdmin) {
    const actingOrgId = cookies().get(ACTING_ORG_COOKIE)?.value;
    if (!actingOrgId) {
      throw new ForbiddenError("Enter a shop from the platform console first");
    }
    return { orgId: actingOrgId, userId: user.id as string, role: "ADMIN" };
  }

  if (!user.orgId) throw new AuthError();
  return {
    orgId: user.orgId as string,
    userId: user.id as string,
    role: user.role as string,
  };
}

// Guard for platform-level (/api/admin/*) routes — super admins only.
export async function requireSuperAdmin(): Promise<{ userId: string }> {
  const session = await getServerSession(authOptions);
  if (!session?.user) throw new AuthError();
  if (!(session.user as any).isSuperAdmin) throw new ForbiddenError();
  return { userId: (session.user as any).id as string };
}

// Backward-compatible alias
export async function getTenantId(): Promise<string> {
  const ctx = await getTenantContext();
  return ctx.orgId;
}

export async function requirePermission(permission: Permission): Promise<TenantContext> {
  const ctx = await getTenantContext();
  const allowed = ROLE_PERMISSIONS[ctx.role] ?? [];
  if (!allowed.includes(permission)) {
    throw new ForbiddenError();
  }
  return ctx;
}

// Response helpers
export function unauthorizedResponse() {
  return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
}

export function forbiddenResponse() {
  return NextResponse.json({ error: "Forbidden" }, { status: 403 });
}

export function errorResponse(message: string, status = 400) {
  return NextResponse.json({ error: message }, { status });
}

// Centralized error handler for catch blocks
export function handleRouteError(error: unknown) {
  if (error instanceof AuthError) {
    return unauthorizedResponse();
  }
  if (error instanceof ForbiddenError) {
    return forbiddenResponse();
  }
  const message = error instanceof Error ? error.message : "Internal server error";
  return errorResponse(message, 500);
}
