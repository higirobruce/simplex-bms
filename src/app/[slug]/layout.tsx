import { SideNav } from "@/components/sidenav";
import { ImpersonationBanner } from "@/components/impersonation-banner";
import { getPlatformSettings } from "@/lib/platform";
import { CurrencyProvider } from "@/lib/currency";
import { prisma } from "@/lib/prisma";

export default async function TenantLayout({
  children,
  params,
}: {
  children: React.ReactNode;
  params: { slug: string };
}) {
  const [{ platformName }, org] = await Promise.all([
    getPlatformSettings(),
    prisma.org.findUnique({ where: { slug: params.slug }, select: { currency: true } }),
  ]);
  return (
    <CurrencyProvider currency={org?.currency ?? "RWF"}>
      <div className="min-h-screen bg-bg">
        <ImpersonationBanner />
        <div className="flex min-h-screen lg:gap-5 lg:p-5">
          <SideNav slug={params.slug} platformName={platformName} />
          <main className="flex-1 overflow-auto">
            <div className="panel min-h-[calc(100vh-2.5rem)] px-5 py-6 sm:px-8 sm:py-8 lg:px-10 lg:py-10">
              {children}
            </div>
          </main>
        </div>
      </div>
    </CurrencyProvider>
  );
}
