import * as React from "react";
import { cn } from "@/lib/utils";

export interface TextareaProps
  extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, ...props }, ref) => {
    return (
      <textarea
        className={cn(
          "flex min-h-[80px] w-full rounded-[var(--radius)] border border-line bg-surface px-3 py-2.5 text-sm text-ink",
          "placeholder:text-ink-mute",
          "transition-all hover:border-line-strong",
          "focus-visible:outline-none focus-visible:border-gold focus-visible:ring-2 focus-visible:ring-gold/30",
          "disabled:cursor-not-allowed disabled:opacity-50",
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Textarea.displayName = "Textarea";

export { Textarea };
