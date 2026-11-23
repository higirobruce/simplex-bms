import argon2 from "argon2";
import { randomBytes } from "node:crypto";

// Cashier accounts are real User rows but never web-login accounts.

// argon2id, 128 MiB, 3 passes, 1 lane. A 6-digit PIN has only 10^6 candidates, so
// the KDF cost is the only protection for a stolen till database: memory is set
// high to make GPU/ASIC guessing expensive. Output is a standard PHC string
// ($argon2id$v=19$m=131072,t=3,p=1$...) so the Rust `argon2` crate can verify it.
export const PIN_ARGON2_OPTIONS = {
  type: argon2.argon2id,
  memoryCost: 131072, // KiB
  timeCost: 3,
  parallelism: 1,
} as const;

export const PIN_MIN_LENGTH = 6;
export const PIN_MAX_LENGTH = 12;

export function validatePin(pin: unknown): pin is string {
  return typeof pin === "string" && new RegExp(`^[0-9]{${PIN_MIN_LENGTH},${PIN_MAX_LENGTH}}$`).test(pin);
}

export async function hashPin(pin: string): Promise<string> {
  if (!validatePin(pin)) throw new Error(`PIN must be ${PIN_MIN_LENGTH}-${PIN_MAX_LENGTH} digits`);
  return argon2.hash(pin, PIN_ARGON2_OPTIONS);
}

/**
 * Value for User.password on a cashier. bcrypt hashes are exactly 60 chars
 * starting with "$2"; bcryptjs.compare returns false for any other length
 * without comparing anything, so no input can ever authenticate against this.
 * It is not derived from the PIN or any secret.
 */
export const UNUSABLE_PASSWORD = "!cashier-no-web-login";

/**
 * Stable (generated once, then stored), unique per org (@@unique([email, orgId])),
 * never shown. A random suffix guarantees uniqueness for duplicate names; the
 * slug is only a readable namespace. The ".invalid" TLD can never receive mail.
 */
export function syntheticEmail(name: string, slug: string): string {
  const clean = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 30) || "x";
  return `cashier.${clean(name)}.${randomBytes(8).toString("hex")}@${clean(slug)}.cashier.invalid`;
}
