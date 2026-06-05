import { withAuth } from "next-auth/middleware";
import { NextResponse } from "next/server";

export default withAuth(
  function middleware(req) {
    const { token } = req.nextauth;
    const path = req.nextUrl.pathname;
    const isSuperAdmin = (token as any)?.isSuperAdmin;
    const orgSlug = (token as any)?.orgSlug;

    // Platform console: super admins only.
    if (path === "/admin" || path.startsWith("/admin/")) {
      if (!isSuperAdmin) {
        const url = req.nextUrl.clone();
        url.pathname = orgSlug ? `/${orgSlug}/dashboard` : "/auth/login";
        return NextResponse.redirect(url);
      }
      return NextResponse.next();
    }

    // A super admin has no shop of their own. Keep them in the console unless
    // they are actively impersonating a shop (acting-org cookie present).
    if (isSuperAdmin && path !== "/") {
      const acting = req.cookies.get("simplex_acting_org");
      if (!acting) {
        const url = req.nextUrl.clone();
        url.pathname = "/admin";
        return NextResponse.redirect(url);
      }
    }

    return NextResponse.next();
  },
  {
    pages: { signIn: "/auth/login" },
  }
);

export const config = {
  matcher: ["/((?!api|_next|auth|favicon.ico).*)"],
};
