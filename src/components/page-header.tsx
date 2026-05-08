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
        "flex flex-col lg:flex-row justify-between items-start lg:items-end gap-6 mb-9 pb-7 border-b border-line",
        className
      )}
    >
      <div>
        {eyebrow && (
          <p className="text-[0.7rem] tracking-[0.22em] uppercase text-gold font-medium mb-3">
            {eyebrow}
          </p>
        )}
        <h1 className="font-serif text-5xl leading-tight text-ink">{title}</h1>
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
