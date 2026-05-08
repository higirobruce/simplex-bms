import * as React from "react";
import { cn } from "@/lib/utils";

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "default"
    | "destructive"
    | "outline"
    | "secondary"
    | "ghost"
    | "link"
    | "gold";
  size?: "default" | "sm" | "lg" | "icon";
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "default", size = "default", ...props }, ref) => {
    const variants: Record<string, string> = {
      default:
        "bg-ink text-surface hover:bg-accent-hover border border-ink",
      gold:
        "bg-gold text-surface hover:brightness-95 border border-gold",
      destructive:
        "bg-danger text-surface hover:brightness-110 border border-danger",
      outline:
        "border border-line-strong bg-surface text-ink hover:bg-surface-2 hover:border-ink",
      secondary:
        "bg-surface-3 text-ink hover:bg-bg-tint border border-line",
      ghost:
        "text-ink-soft hover:bg-surface-2 hover:text-ink",
      link:
        "text-gold underline-offset-4 hover:underline",
    };
    const sizes: Record<string, string> = {
      default: "h-10 px-4 py-2",
      sm: "h-8 rounded-[var(--radius)] px-3 text-[0.78rem]",
      lg: "h-11 rounded-[var(--radius)] px-6",
      icon: "h-10 w-10",
    };
    return (
      <button
        className={cn(
          "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-[var(--radius)] text-sm font-semibold tracking-wide transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-gold/50 focus-visible:ring-offset-2 focus-visible:ring-offset-bg disabled:pointer-events-none disabled:opacity-50 uppercase",
          variants[variant],
          sizes[size],
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Button.displayName = "Button";

export { Button };
