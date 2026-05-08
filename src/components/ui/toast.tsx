"use client";

import * as React from "react";
import { cn } from "@/lib/utils";
import { X, CheckCircle2, AlertCircle, Info } from "lucide-react";

type ToastType = "success" | "error" | "info";

interface Toast {
  id: string;
  type: ToastType;
  message: string;
}

interface ToastContextValue {
  toast: (type: ToastType, message: string) => void;
  success: (message: string) => void;
  error: (message: string) => void;
  info: (message: string) => void;
}

const ToastContext = React.createContext<ToastContextValue | null>(null);

export function useToast() {
  const ctx = React.useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = React.useState<Toast[]>([]);

  const removeToast = React.useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const addToast = React.useCallback(
    (type: ToastType, message: string) => {
      const id = Math.random().toString(36).slice(2);
      setToasts((prev) => [...prev.slice(-4), { id, type, message }]);
      setTimeout(() => removeToast(id), 4000);
    },
    [removeToast]
  );

  const value = React.useMemo(
    () => ({
      toast: addToast,
      success: (msg: string) => addToast("success", msg),
      error: (msg: string) => addToast("error", msg),
      info: (msg: string) => addToast("info", msg),
    }),
    [addToast]
  );

  const icons: Record<ToastType, React.ReactNode> = {
    success: <CheckCircle2 className="h-4 w-4 text-success" strokeWidth={1.75} />,
    error: <AlertCircle className="h-4 w-4 text-danger" strokeWidth={1.75} />,
    info: <Info className="h-4 w-4 text-info" strokeWidth={1.75} />,
  };

  const accentBars: Record<ToastType, string> = {
    success: "bg-success",
    error: "bg-danger",
    info: "bg-info",
  };

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="fixed bottom-5 right-5 z-[100] flex flex-col gap-2.5 w-[22rem]">
        {toasts.map((t) => (
          <div
            key={t.id}
            className="relative flex items-start gap-3 rounded-[var(--radius)] border border-line bg-surface p-4 pr-9 shadow-lift overflow-hidden animate-slide-in-right"
          >
            <span
              className={cn(
                "absolute left-0 top-0 bottom-0 w-1",
                accentBars[t.type]
              )}
            />
            <span className="mt-0.5 ml-1">{icons[t.type]}</span>
            <p className="flex-1 text-sm leading-relaxed text-ink">
              {t.message}
            </p>
            <button
              onClick={() => removeToast(t.id)}
              className="absolute top-3 right-3 text-ink-mute hover:text-ink rounded-full p-1 hover:bg-surface-2 transition-colors"
              aria-label="Dismiss"
            >
              <X className="h-3.5 w-3.5" strokeWidth={1.75} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
