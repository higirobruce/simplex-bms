import * as React from "react";
import { cn } from "@/lib/utils";

interface PageHeaderProps {
  eyebrow?: string;
  title: string;
  description?: string;
  actions?: React.ReactNode;
  className?: string;
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
  className,
}: PageHeaderProps) {
  return (
    <div
      className={cn(
        "flex flex-col lg:flex-row justify-between items-start lg:items-end gap-6 mb-8 pb-6 border-b border-line",
        className
      )}
    >
      <div>
        {eyebrow && (
          <p className="font-mono text-[0.65rem] tracking-[0.22em] uppercase text-gold font-semibold mb-3 flex items-center gap-2">
            <span className="inline-block h-[2px] w-6 bg-gold" />
            {eyebrow}
          </p>
        )}
        <h1 className="font-display text-4xl sm:text-5xl font-bold tracking-tight text-ink leading-[1.05] uppercase">
          {title}
        </h1>
        {description && (
          <p className="mt-3 text-ink-soft max-w-xl leading-relaxed">
            {description}
          </p>
        )}
      </div>
      {actions && <div className="flex items-center gap-3">{actions}</div>}
    </div>
  );
}
