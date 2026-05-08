import * as React from "react";
import { cn } from "@/lib/utils";

export interface InputProps
  extends React.InputHTMLAttributes<HTMLInputElement> {}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          "flex h-11 w-full rounded-[var(--radius)] border border-line bg-surface-2/60 px-4 py-2 text-sm text-ink",
          "placeholder:text-ink-mute",
          "transition-all",
          "hover:border-line-strong",
          "focus-visible:outline-none focus-visible:bg-surface focus-visible:border-accent/40 focus-visible:ring-4 focus-visible:ring-accent/10",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-ink",
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Input.displayName = "Input";

export { Input };
