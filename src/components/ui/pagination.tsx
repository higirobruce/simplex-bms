"use client";

import {
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
} from "lucide-react";
import { cn } from "@/lib/utils";

interface PaginationProps {
  page: number;
  totalPages: number;
  onPageChange: (page: number) => void;
}

const buttonClass =
  "inline-flex h-9 w-9 items-center justify-center rounded-full border border-line text-ink-soft transition-all hover:bg-surface-2 hover:text-ink hover:border-line-strong disabled:opacity-40 disabled:cursor-not-allowed";

export function Pagination({
  page,
  totalPages,
  onPageChange,
}: PaginationProps) {
  if (totalPages <= 1) return null;

  return (
    <div className="flex items-center justify-between mt-6 pt-5 border-t border-line">
      <p className="text-sm text-ink-soft">
        Showing page <span className="font-medium text-ink tabular">{page}</span>{" "}
        of <span className="font-medium text-ink tabular">{totalPages}</span>
      </p>
      <div className="flex items-center gap-1.5">
        <button
          className={buttonClass}
          onClick={() => onPageChange(1)}
          disabled={page <= 1}
          aria-label="First page"
        >
          <ChevronsLeft className="h-4 w-4" strokeWidth={1.75} />
        </button>
        <button
          className={buttonClass}
          onClick={() => onPageChange(page - 1)}
          disabled={page <= 1}
          aria-label="Previous page"
        >
          <ChevronLeft className="h-4 w-4" strokeWidth={1.75} />
        </button>
        <button
          className={buttonClass}
          onClick={() => onPageChange(page + 1)}
          disabled={page >= totalPages}
          aria-label="Next page"
        >
          <ChevronRight className="h-4 w-4" strokeWidth={1.75} />
        </button>
        <button
          className={buttonClass}
          onClick={() => onPageChange(totalPages)}
          disabled={page >= totalPages}
          aria-label="Last page"
        >
          <ChevronsRight className="h-4 w-4" strokeWidth={1.75} />
        </button>
      </div>
    </div>
  );
}
