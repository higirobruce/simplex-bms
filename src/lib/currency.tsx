"use client";

import { createContext, useContext } from "react";
import { formatCurrency } from "@/lib/utils";

// Active shop currency, provided by the tenant layout. Defaults to RWF so the
// formatter is always safe even outside a provider.
const CurrencyContext = createContext<string>("RWF");

export function CurrencyProvider({
  currency,
  children,
}: {
  currency: string;
  children: React.ReactNode;
}) {
  return <CurrencyContext.Provider value={currency}>{children}</CurrencyContext.Provider>;
}

export function useCurrency(): string {
  return useContext(CurrencyContext);
}

// Returns a formatter bound to the active shop's currency, e.g. fmt(1500).
export function useMoney() {
  const currency = useContext(CurrencyContext);
  return (amount: number | string) => formatCurrency(amount, currency);
}
