import { z } from "zod";

// ===== Org =====
export const OrgPatchSchema = z.object({
  name: z.string().min(1, "Organisation name is required").optional(),
  logo: z.string().url("Logo must be a valid URL").optional().nullable(),
  currency: z.string().min(1).max(8).optional(),
  timezone: z.string().min(1).optional(),
});

// ===== Platform admin: Shops (orgs) =====
const slugRegex = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

export const ShopCreateSchema = z.object({
  name: z.string().min(1, "Shop name is required"),
  slug: z
    .string()
    .min(2, "Slug must be at least 2 characters")
    .max(40)
    .regex(slugRegex, "Slug may contain lowercase letters, numbers and hyphens only"),
  currency: z.string().min(1).max(8).default("RWF"),
  timezone: z.string().min(1).default("Africa/Kigali"),
  // First admin user for the new shop
  adminEmail: z.string().email("Invalid admin email"),
  adminName: z.string().min(1).optional().nullable(),
  adminPassword: z.string().min(8, "Password must be at least 8 characters"),
});

export const ShopPatchSchema = z.object({
  name: z.string().min(1).optional(),
  logo: z.string().url("Logo must be a valid URL").optional().nullable(),
  currency: z.string().min(1).max(8).optional(),
  timezone: z.string().min(1).optional(),
  status: z.enum(["ACTIVE", "SUSPENDED"]).optional(),
});

// ===== Platform settings =====
export const PlatformSettingsSchema = z.object({
  platformName: z.string().min(1).optional(),
  supportEmail: z.string().email("Invalid email").optional().nullable(),
  defaultCurrency: z.string().min(1).max(8).optional(),
  defaultTimezone: z.string().min(1).optional(),
  allowSignups: z.boolean().optional(),
});

// ===== Users =====
export const UserCreateSchema = z.object({
  email: z.string().email("Invalid email"),
  name: z.string().min(1, "Name is required").optional().nullable(),
  password: z.string().min(8, "Password must be at least 8 characters"),
  role: z.enum(["ADMIN", "MANAGER", "ACCOUNTANT", "VIEWER"]).default("VIEWER"),
});

export const UserPatchSchema = z.object({
  name: z.string().min(1).optional().nullable(),
  role: z.enum(["ADMIN", "MANAGER", "ACCOUNTANT", "VIEWER"]).optional(),
  password: z.string().min(8, "Password must be at least 8 characters").optional(),
});

// ===== Products =====
export const ProductCreateSchema = z.object({
  sku: z.string().min(1, "SKU is required"),
  name: z.string().min(1, "Product name is required"),
  description: z.string().optional(),
  unitPrice: z.number().positive("Unit price must be positive"),
  costPrice: z.number().positive("Cost price must be positive").optional().nullable(),
  category: z.string().default("General"),
  reorderLevel: z.number().int().min(0).default(10),
});

export const ProductPatchSchema = z.object({
  name: z.string().min(1).optional(),
  description: z.string().optional().nullable(),
  unitPrice: z.number().positive().optional(),
  costPrice: z.number().positive().optional().nullable(),
  category: z.string().optional(),
  reorderLevel: z.number().int().min(0).optional(),
});
// SKU excluded — immutable after creation

// ===== Bulk import =====
// CSV cells arrive as strings, so coerce numbers and treat blanks as omitted.
const blankToUndef = (v: unknown) => (v === "" || v === null ? undefined : v);
export const ProductImportRowSchema = z.object({
  sku: z.string().trim().min(1, "SKU is required"),
  name: z.string().trim().min(1, "Name is required"),
  category: z.preprocess(blankToUndef, z.string().trim().default("General")),
  unitPrice: z.coerce.number().positive("Unit price must be positive"),
  costPrice: z.preprocess(blankToUndef, z.coerce.number().positive().optional()),
  reorderLevel: z.preprocess(blankToUndef, z.coerce.number().int().min(0).default(10)),
});

// Generic envelope for any CSV import — an array of raw row objects.
export const BulkImportSchema = z.object({
  rows: z.array(z.record(z.string(), z.any())).min(1, "No rows to import").max(1000, "Too many rows (max 1000)"),
});

export const CustomerImportRowSchema = z.object({
  name: z.string().trim().min(1, "Name is required"),
  email: z.preprocess(blankToUndef, z.string().email("Invalid email").optional()),
  phone: z.preprocess(blankToUndef, z.string().trim().optional()),
  address: z.preprocess(blankToUndef, z.string().trim().optional()),
  city: z.preprocess(blankToUndef, z.string().trim().optional()),
  country: z.preprocess(blankToUndef, z.string().trim().optional()),
  creditLimit: z.preprocess(blankToUndef, z.coerce.number().min(0).default(0)),
});

export const StockImportRowSchema = z.object({
  sku: z.string().trim().min(1, "SKU is required"),
  location: z.string().trim().min(1, "Location is required"),
  qty: z.coerce.number().int().positive("Qty must be a positive whole number"),
});

export const VendorImportRowSchema = z.object({
  name: z.string().trim().min(1, "Name is required"),
  email: z.preprocess(blankToUndef, z.string().email("Invalid email").optional()),
  phone: z.preprocess(blankToUndef, z.string().trim().optional()),
  address: z.preprocess(blankToUndef, z.string().trim().optional()),
  city: z.preprocess(blankToUndef, z.string().trim().optional()),
  country: z.preprocess(blankToUndef, z.string().trim().optional()),
  paymentTerms: z.preprocess(blankToUndef, z.string().trim().default("NET30")),
});

// ===== Customers =====
export const CustomerCreateSchema = z.object({
  name: z.string().min(1, "Customer name is required"),
  email: z.string().email("Invalid email").optional().nullable(),
  phone: z.string().optional().nullable(),
  address: z.string().optional().nullable(),
  city: z.string().optional().nullable(),
  country: z.string().optional().nullable(),
  creditLimit: z.number().min(0).default(0),
});

export const CustomerPatchSchema = z.object({
  name: z.string().min(1).optional(),
  email: z.string().email().optional().nullable(),
  phone: z.string().optional().nullable(),
  address: z.string().optional().nullable(),
  city: z.string().optional().nullable(),
  country: z.string().optional().nullable(),
  creditLimit: z.number().min(0).optional(),
  status: z.enum(["ACTIVE", "INACTIVE"]).optional(),
});

// ===== Vendors =====
export const VendorCreateSchema = z.object({
  name: z.string().min(1, "Vendor name is required"),
  email: z.string().email("Invalid email").optional().nullable(),
  phone: z.string().optional().nullable(),
  address: z.string().optional().nullable(),
  city: z.string().optional().nullable(),
  country: z.string().optional().nullable(),
  paymentTerms: z.string().default("NET30"),
});

export const VendorPatchSchema = z.object({
  name: z.string().min(1).optional(),
  email: z.string().email().optional().nullable(),
  phone: z.string().optional().nullable(),
  address: z.string().optional().nullable(),
  city: z.string().optional().nullable(),
  country: z.string().optional().nullable(),
  paymentTerms: z.string().optional(),
  status: z.enum(["ACTIVE", "INACTIVE"]).optional(),
});

// ===== Expenses =====
export const ExpenseCreateSchema = z.object({
  category: z.string().min(1, "Category is required"),
  amount: z.number().positive("Amount must be positive"),
  date: z.string().optional(),
  description: z.string().optional().nullable(),
});

export const ExpensePatchSchema = z.object({
  category: z.string().min(1).optional(),
  amount: z.number().positive().optional(),
  date: z.string().optional(),
  description: z.string().optional().nullable(),
});

// ===== Invoices =====
const InvoiceLineItemSchema = z.object({
  productId: z.string().optional().nullable(),
  description: z.string().optional().nullable(),
  qty: z.number().int().min(1, "Quantity must be at least 1"),
  unitPrice: z.number().min(0, "Unit price cannot be negative"),
});

export const InvoiceCreateSchema = z.object({
  customerId: z.string().min(1, "Customer is required"),
  lineItems: z.array(InvoiceLineItemSchema).min(1, "At least one line item is required"),
  dueDate: z.string().min(1, "Due date is required"),
  notes: z.string().optional().nullable(),
});

export const InvoicePatchSchema = z.object({
  notes: z.string().optional().nullable(),
  dueDate: z.string().optional(),
});

// ===== Payments =====
export const PaymentCreateSchema = z.object({
  amount: z.number().positive("Payment amount must be positive"),
  method: z.enum(["CASH", "MOBILE_MONEY", "BANK", "CHECK", "OTHER"]),
  notes: z.string().optional().nullable(),
});

// ===== Stock =====
export const StockAdjustSchema = z.object({
  productId: z.string().min(1, "Product is required"),
  locationId: z.string().min(1, "Location is required"),
  qty: z.number().int().refine((v) => v !== 0, "Quantity cannot be zero"),
  type: z.enum(["RECEIPT", "ISSUE", "TRANSFER", "ADJUSTMENT", "RETURN"]),
  reason: z.string().optional().nullable(),
});

// ===== Invoice Status Transitions =====
export const InvoiceStatusTransitionSchema = z.object({
  action: z.enum(["send", "mark_overdue", "void", "reopen"]),
});

// ===== Pagination =====
export const ListQuerySchema = z.object({
  page: z.coerce.number().int().min(1).default(1),
  limit: z.coerce.number().int().min(1).max(100).default(20),
  search: z.string().optional(),
  status: z.string().optional(),
  sortBy: z.string().default("createdAt"),
  sortOrder: z.enum(["asc", "desc"]).default("desc"),
});

// ===== Locations / Warehouses =====
export const LocationCreateSchema = z.object({
  name: z.string().min(1, "Location name is required"),
  code: z.string().optional().nullable(),
  address: z.string().optional().nullable(),
  parentId: z.string().optional().nullable(),
});

export const LocationPatchSchema = z.object({
  name: z.string().min(1).optional(),
  code: z.string().optional().nullable(),
  address: z.string().optional().nullable(),
  parentId: z.string().optional().nullable(),
});

// ===== Sales =====
const OrderLineSchema = z.object({
  productId: z.string().min(1, "Product is required"),
  qty: z.number().int().positive("Qty must be positive"),
  unitPrice: z.number().min(0, "Unit price cannot be negative"),
});

export const SalesOrderCreateSchema = z.object({
  customerId: z.string().min(1, "Customer is required"),
  expectedDate: z.string().optional().nullable(),
  notes: z.string().optional().nullable(),
  lines: z.array(OrderLineSchema).min(1, "At least one line is required"),
});

export const SalesOrderPatchSchema = z.object({
  customerId: z.string().min(1).optional(),
  expectedDate: z.string().optional().nullable(),
  notes: z.string().optional().nullable(),
  lines: z.array(OrderLineSchema).min(1).optional(),
});

const DeliveryNoteLineSchema = z.object({
  productId: z.string().min(1),
  orderLineId: z.string().optional().nullable(),
  qty: z.number().int().positive(),
});

export const DeliveryNoteCreateSchema = z.object({
  orderId: z.string().min(1, "Sales order is required"),
  locationId: z.string().min(1, "Source warehouse is required"),
  notes: z.string().optional().nullable(),
  lines: z.array(DeliveryNoteLineSchema).min(1, "At least one line is required"),
});

// ===== Procurement =====
export const PurchaseOrderCreateSchema = z.object({
  vendorId: z.string().min(1, "Vendor is required"),
  expectedDate: z.string().optional().nullable(),
  notes: z.string().optional().nullable(),
  lines: z.array(OrderLineSchema).min(1, "At least one line is required"),
});

export const PurchaseOrderPatchSchema = z.object({
  vendorId: z.string().min(1).optional(),
  expectedDate: z.string().optional().nullable(),
  notes: z.string().optional().nullable(),
  lines: z.array(OrderLineSchema).min(1).optional(),
});

const GoodsReceiptLineSchema = z.object({
  productId: z.string().min(1),
  orderLineId: z.string().optional().nullable(),
  qty: z.number().int().positive(),
});

export const GoodsReceiptCreateSchema = z.object({
  orderId: z.string().min(1, "Purchase order is required"),
  locationId: z.string().min(1, "Destination warehouse is required"),
  notes: z.string().optional().nullable(),
  lines: z.array(GoodsReceiptLineSchema).min(1, "At least one line is required"),
});
