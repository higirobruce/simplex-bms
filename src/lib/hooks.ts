"use client";

import { useState, useEffect } from "react";
import { useSession } from "next-auth/react";
import { useParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";

export interface PublicPlatform {
  platformName: string;
  supportEmail: string | null;
  defaultCurrency: string;
  defaultTimezone: string;
  allowSignups: boolean;
}

// Public platform settings (branding, signup gating, shop defaults).
export function usePlatform(): PublicPlatform | undefined {
  const { data } = useQuery<PublicPlatform>({
    queryKey: ["platform"],
    queryFn: () => fetch("/api/platform").then((r) => r.json()),
    staleTime: 5 * 60 * 1000,
  });
  return data;
}

export function useTenant() {
  const { data: session } = useSession();
  const params = useParams();
  const slug = params?.slug as string;

  return {
    slug,
    orgId: (session?.user as any)?.orgId as string | undefined,
    orgSlug: (session?.user as any)?.orgSlug as string | undefined,
    role: (session?.user as any)?.role as string | undefined,
    user: session?.user,
  };
}

export function useDebounce<T>(value: T, delay: number): T {
  const [debouncedValue, setDebouncedValue] = useState<T>(value);

  useEffect(() => {
    const handler = setTimeout(() => setDebouncedValue(value), delay);
    return () => clearTimeout(handler);
  }, [value, delay]);

  return debouncedValue;
}
