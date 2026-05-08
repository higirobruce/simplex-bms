"use client";

import { useState, useEffect } from "react";
import { useSession } from "next-auth/react";
import { useParams } from "next/navigation";

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
