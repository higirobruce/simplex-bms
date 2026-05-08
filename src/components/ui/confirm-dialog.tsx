"use client";

import { Dialog, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { AlertTriangle } from "lucide-react";

interface ConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: string;
  description: string;
  confirmLabel?: string;
  variant?: "destructive" | "default";
  loading?: boolean;
}

export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  description,
  confirmLabel = "Confirm",
  variant = "default",
  loading = false,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onClose={onClose}>
      <DialogHeader>
        <div className="flex items-center gap-4">
          {variant === "destructive" && (
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-danger-soft">
              <AlertTriangle
                className="h-5 w-5 text-danger"
                strokeWidth={1.75}
              />
            </div>
          )}
          <DialogTitle>{title}</DialogTitle>
        </div>
      </DialogHeader>
      <p className="text-sm text-ink-soft leading-relaxed mb-7">
        {description}
      </p>
      <div className="flex justify-end gap-3">
        <Button variant="outline" onClick={onClose} disabled={loading}>
          Cancel
        </Button>
        <Button
          variant={variant === "destructive" ? "destructive" : "default"}
          onClick={onConfirm}
          disabled={loading}
        >
          {loading ? "Please wait…" : confirmLabel}
        </Button>
      </div>
    </Dialog>
  );
}
