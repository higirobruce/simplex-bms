"use client";

import { Button } from "@/components/ui/button";
import { AlertCircle } from "lucide-react";

export default function TenantError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <div className="flex flex-col items-center justify-center min-h-[60vh] px-8 text-center">
      <div className="h-14 w-14 rounded-full bg-danger-soft flex items-center justify-center mb-6">
        <AlertCircle
          className="h-6 w-6 text-danger"
          strokeWidth={1.75}
        />
      </div>
      <h2 className="font-display text-3xl font-bold uppercase tracking-tight text-ink mb-3">
        A small disturbance.
      </h2>
      <p className="text-ink-soft mb-7 max-w-md leading-relaxed">
        {error.message ||
          "Something didn't quite go to plan. Please try again — or refresh the page."}
      </p>
      <Button onClick={reset}>Try again</Button>
    </div>
  );
}
