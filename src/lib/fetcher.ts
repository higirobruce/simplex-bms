// Shared client-side JSON fetch helper. Throws Error(message) on non-2xx.
export function apiCall(url: string, opts?: RequestInit) {
  return fetch(url, {
    ...opts,
    headers: { "Content-Type": "application/json", ...opts?.headers },
  }).then(async (r) => {
    if (!r.ok) {
      const d = await r.json().catch(() => ({}));
      throw new Error(d.error || "Request failed");
    }
    return r.json();
  });
}
