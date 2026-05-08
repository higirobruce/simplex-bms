export function activeWhere(orgId: string) {
  return { orgId, deletedAt: null };
}

export function activeProductWhere(orgId: string) {
  return { orgId, deletedAt: null };
}

export function activeCustomerWhere(orgId: string) {
  return { orgId, deletedAt: null };
}

export function activeVendorWhere(orgId: string) {
  return { orgId, deletedAt: null };
}

export function activeInvoiceWhere(orgId: string) {
  return { orgId, deletedAt: null };
}
