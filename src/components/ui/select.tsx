import * as React from "react";
import { cn } from "@/lib/utils";

export interface SelectProps
  extends React.SelectHTMLAttributes<HTMLSelectElement> {}

const Select = React.forwardRef<HTMLSelectElement, SelectProps>(
  ({ className, children, ...props }, ref) => {
    return (
      <select
        ref={ref}
        className={cn(
          "flex h-11 w-full rounded-[var(--radius)] border border-line bg-surface-2/60 px-4 py-2 text-sm text-ink appearance-none",
          "bg-no-repeat bg-[right_0.85rem_center]",
          "transition-all hover:border-line-strong",
          "focus-visible:outline-none focus-visible:bg-surface focus-visible:border-accent/40 focus-visible:ring-4 focus-visible:ring-accent/10",
          "disabled:cursor-not-allowed disabled:opacity-50",
          className
        )}
        style={{
          backgroundImage:
            "url(\"data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 12 12'><path fill='none' stroke='%2357504a' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round' d='M3 4.5L6 7.5L9 4.5'/></svg>\")",
          backgroundSize: "12px 12px",
          paddingRight: "2.25rem",
        }}
        {...props}
      >
        {children}
      </select>
    );
  }
);
Select.displayName = "Select";

export { Select };
