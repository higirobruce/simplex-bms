import { redirect } from "next/navigation";
import { getServerSession } from "next-auth";
import { authOptions } from "@/lib/auth";

export default async function HomePage() {
  const session = await getServerSession(authOptions);

  if (session?.user) {
    if ((session.user as any).isSuperAdmin) {
      redirect("/admin");
    }
    redirect(`/${(session.user as any).orgSlug}/dashboard`);
  }

  redirect("/auth/login");
}
