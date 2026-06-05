"use client";

import { useState, useRef, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { useDebounce } from "@/lib/hooks";
import { cn } from "@/lib/utils";
import { Search, Check, ChevronDown } from "lucide-react";

interface AsyncComboboxProps<T> {
  /** Currently selected value (e.g. an id). Empty string = nothing selected. */
  value: string;
  /** Label to display for the current value (controlled by the parent). */
  selectedLabel?: string;
  /** API path to search, e.g. `/api/${slug}/products`. */
  endpoint: string;
  /** React-Query key prefix; the search term is appended automatically. */
  queryKey: (string | number)[];
  /** Render an option's display label. */
  getLabel: (option: T) => string;
  /** Extract an option's value; defaults to `option.id`. */
  getValue?: (option: T) => string;
  onSelect: (value: string, option: T) => void;
  placeholder?: string;
  disabled?: boolean;
  required?: boolean;
}

export function AsyncCombobox<T extends Record<string, any>>({
  value,
  selectedLabel,
  endpoint,
  queryKey,
  getLabel,
  getValue = (o) => o.id,
  onSelect,
  placeholder = "Search…",
  disabled,
  required,
}: AsyncComboboxProps<T>) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const debounced = useDebounce(search, 250);
  const wrapRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [open]);

  // Focus the search box when opening.
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const sep = endpoint.includes("?") ? "&" : "?";
  const { data, isLoading } = useQuery({
    queryKey: [...queryKey, debounced],
    queryFn: () =>
      fetch(`${endpoint}${sep}limit=20&search=${encodeURIComponent(debounced)}`).then((r) => r.json()),
    enabled: open,
  });
  const options: T[] = Array.isArray(data) ? data : data?.data ?? [];

  return (
    <div className="relative" ref={wrapRef}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
        className={cn(
          "flex h-10 w-full items-center justify-between gap-2 rounded-[var(--radius)] border border-line bg-surface px-3 py-2 text-left text-sm transition-all hover:border-line-strong",
          "focus-visible:outline-none focus-visible:border-gold focus-visible:ring-2 focus-visible:ring-gold/30",
          "disabled:cursor-not-allowed disabled:opacity-50"
        )}
      >
        <span className={cn("truncate", selectedLabel ? "text-ink" : "text-ink-mute")}>
          {selectedLabel || placeholder}
        </span>
        <ChevronDown className="h-3.5 w-3.5 shrink-0 text-ink-mute" strokeWidth={1.75} />
      </button>

      {/* Validation hook: an empty hidden input keeps native `required` working. */}
      {required && (
        <input
          tabIndex={-1}
          aria-hidden
          required
          value={value}
          onChange={() => {}}
          className="absolute h-0 w-0 opacity-0"
        />
      )}

      {open && (
        <div className="absolute z-50 mt-1 w-full overflow-hidden rounded-[var(--radius)] border border-line bg-surface shadow-panel">
          <div className="relative border-b border-line">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-ink-mute" strokeWidth={1.75} />
            <input
              ref={inputRef}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={placeholder}
              className="h-10 w-full bg-transparent pl-9 pr-3 text-sm text-ink placeholder:text-ink-mute focus:outline-none"
            />
          </div>
          <ul className="max-h-60 overflow-y-auto py-1">
            {isLoading ? (
              <li className="px-3 py-3 text-center font-mono text-[0.65rem] uppercase tracking-[0.18em] text-ink-mute">
                Searching…
              </li>
            ) : options.length === 0 ? (
              <li className="px-3 py-3 text-center text-sm text-ink-mute">No matches.</li>
            ) : (
              options.map((opt) => {
                const v = getValue(opt);
                const selected = v === value;
                return (
                  <li key={v}>
                    <button
                      type="button"
                      onClick={() => {
                        onSelect(v, opt);
                        setOpen(false);
                        setSearch("");
                      }}
                      className={cn(
                        "flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-sm transition-colors hover:bg-surface-2",
                        selected ? "text-ink font-medium" : "text-ink-soft"
                      )}
                    >
                      <span className="truncate">{getLabel(opt)}</span>
                      {selected && <Check className="h-3.5 w-3.5 shrink-0 text-gold" strokeWidth={2} />}
                    </button>
                  </li>
                );
              })
            )}
          </ul>
        </div>
      )}
    </div>
  );
}
