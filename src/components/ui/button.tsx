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
        "bg-accent text-surface hover:bg-accent-hover shadow-sm",
      gold:
        "bg-gold text-surface hover:brightness-95 shadow-sm",
      destructive:
        "bg-danger text-surface hover:brightness-110 shadow-sm",
      outline:
        "border border-line-strong bg-surface text-ink hover:bg-surface-2",
      secondary:
        "bg-surface-3 text-ink hover:bg-bg-tint",
      ghost:
        "text-ink-soft hover:bg-surface-2 hover:text-ink",
      link:
        "text-accent underline-offset-4 hover:underline",
    };
    const sizes: Record<string, string> = {
      default: "h-10 px-5 py-2",
      sm: "h-9 rounded-[var(--radius)] px-3.5 text-[0.8rem]",
      lg: "h-12 rounded-[var(--radius)] px-7",
      icon: "h-10 w-10",
    };
    return (
      <button
        className={cn(
          "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-[var(--radius)] text-sm font-medium tracking-tight transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 focus-visible:ring-offset-2 focus-visible:ring-offset-bg disabled:pointer-events-none disabled:opacity-50",
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
