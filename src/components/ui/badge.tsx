import * as React from "react";
import { cn } from "@/lib/utils";

export interface BadgeProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?:
    | "default"
    | "secondary"
    | "destructive"
    | "outline"
    | "success"
    | "warning"
    | "info"
    | "gold";
}

function Badge({ className, variant = "default", ...props }: BadgeProps) {
  const variants: Record<string, string> = {
    default: "bg-accent-soft text-accent",
    secondary: "bg-surface-3 text-ink-soft",
    destructive: "bg-danger-soft text-danger",
    outline: "border border-line text-ink-soft bg-surface",
    success: "bg-success-soft text-success",
    warning: "bg-warning-soft text-warning",
    info: "bg-info-soft text-info",
    gold: "bg-gold-soft text-gold",
  };
  return (
    <div
      className={cn(
        "pill",
        variants[variant],
        className
      )}
      {...props}
    />
  );
}

export { Badge };
