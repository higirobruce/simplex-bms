import { AdminNav } from "@/components/admin-nav";
import { getPlatformSettings } from "@/lib/platform";

export default async function AdminLayout({ children }: { children: React.ReactNode }) {
  const { platformName } = await getPlatformSettings();
  return (
    <div className="min-h-screen bg-bg">
      <div className="flex min-h-screen lg:gap-5 lg:p-5">
        <AdminNav platformName={platformName} />
        <main className="flex-1 overflow-auto">
          <div className="panel min-h-[calc(100vh-2.5rem)] px-5 py-6 sm:px-8 sm:py-8 lg:px-10 lg:py-10">
            {children}
          </div>
        </main>
      </div>
    </div>
  );
}
