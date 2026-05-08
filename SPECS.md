# SPECKIT: Business Management System (BMS)
## Stock, Finance, Customer & Vendor Management

**Version:** 1.0  
**Status:** MVP Planning  
**Target:** SMBs, Solo Entrepreneurs, Small Retail & Trading Businesses  

---

## 1. PRODUCT OVERVIEW

### Vision
A lightweight, intuitive business management platform that handles the essential operations of small-to-medium businesses: what you have (stock), what you owe/earn (finance), who you work with (customers & vendors). Designed to be operable by non-technical users while remaining modern, fast, and mobile-friendly.

### Core Principle
**"Simplicity First, Power Second"** — Default screens show what matters most; advanced features don't clutter the UI.

---

## 2. CORE MODULES

### 2.1 STOCK MANAGEMENT
**Purpose:** Track inventory in real-time across locations.

**Key Entities:**
- **Products** — SKU, name, category, unit price, reorder level, description, images
- **Stock Levels** — Real-time qty per location/warehouse
- **Stock Movements** — Purchase, sale, adjustment, transfer, write-off (audit trail)
- **Locations** — Multiple warehouses/stores per tenant

**Core Features:**
- Dashboard: Low stock alerts, top movers, inventory value
- Add/Update Products (single form, image upload)
- Stock Adjustment (manual count corrections with reason)
- Stock Transfers (between locations)
- Search & Filter (by category, SKU, stock status)
- Basic Reports: Stock value, movement history, category breakdown
- Barcode/QR code support (generate & scan—read-only for MVP)

**Mobile Considerations:**
- Simplified stock adjustment UI for field use
- Quick search by SKU or name
- Offline sync (optional for MVP+)

---

### 2.2 CUSTOMER MANAGEMENT
**Purpose:** Maintain customer info, track relationships, and credit history.

**Key Entities:**
- **Customers** — Name, contact info, address, credit limit, credit balance, category (retail, wholesale, etc.)
- **Customer Transactions** — Linked to invoices & payments
- **Contact History** — Notes, interaction log (optional for MVP)

**Core Features:**
- Customer List with search/filter (active/inactive, credit status)
- Add/Edit Customer (single form: name, email, phone, address, credit limit)
- Customer Dashboard — Total purchases, balance due, last transaction, credit utilization
- Transaction History — All sales & payments linked to this customer
- Bulk Actions — Deactivate, set credit limits, export
- Simple Contact Info Card (printable/shareable)

**Mobile Considerations:**
- Quick customer lookup during transactions
- Tap to call/email/message
- Customer mini-dashboard (balance due, recent activity)

---

### 2.3 VENDOR MANAGEMENT
**Purpose:** Maintain supplier info and track purchases.

**Key Entities:**
- **Vendors** — Name, contact info, address, payment terms, preferred items
- **Vendor Transactions** — Linked to purchase orders & payments

**Core Features:**
- Vendor List with search/filter (active, payment status)
- Add/Edit Vendor (single form: name, email, phone, address, payment terms)
- Vendor Dashboard — Total purchases, balance owed, last transaction, payment terms
- Transaction History — All purchase orders & payments
- Payment Tracking — Due dates, overdue alerts
- Bulk Actions — Deactivate, update terms, export

---

### 2.4 FINANCIAL MANAGEMENT
**Purpose:** Track revenue, expenses, and cash flow with multi-currency support.

**Key Entities:**
- **Invoices** (Sales) — Customer, items, amounts, status, payment terms, due date
- **Purchase Orders** (Buying) — Vendor, items, amounts, status, payment terms, due date
- **Payments** (Receivable) — Linked to invoices; cash, mobile money, bank transfer
- **Expense Tracking** — Non-stock expenses (rent, utilities, salaries, etc.)
- **Chart of Accounts** (simple) — Prebuilt categories: Revenue, COGS, Expenses, Assets, Liabilities, Equity
- **Currency** — Default to RWF; support USD, EUR, UGX, KES, USD for multi-currency orgs

**Core Features:**

**Invoicing:**
- Create Invoice (customer, select items from stock, manual line items)
- Invoice Template (customizable header with business logo, colors)
- Status Tracking (draft, sent, partially paid, paid, overdue)
- Partial Payments (apply multiple payments to one invoice)
- Batch Send (email invoices)
- Print/Download as PDF

**Payments:**
- Record Payment (against invoice, auto-link by customer or manual)
- Multiple Payment Methods (cash, mobile money, bank, check)
- Exchange Rate Management (for multi-currency)
- Reconciliation (match payments to invoices)

**Expenses:**
- Log Expense (category, amount, date, description, attachment)
- Recurring Expenses (rent, subscriptions)
- Expense Approval (optional workflow for MVP+)

**Financial Dashboard:**
- Revenue (MTD, YTD, trend chart)
- Expenses (MTD breakdown by category)
- Profit Margin (if COGS tracked)
- Outstanding Receivables (from customers)
- Outstanding Payables (to vendors)
- Cash Balance (by currency)
- Quick Stats (invoices sent, paid, overdue)

**Reports:**
- Profit & Loss (by period)
- Balance Sheet (simplified)
- Receivables Aging (invoices due)
- Payables Aging (bills due)
- Revenue by Customer (top customers, trends)
- Expense Breakdown (by category, over time)
- Cash Flow (simple: inflows vs. outflows by period)

**Mobile Considerations:**
- Quick invoice creation (camera photo capture for receipts)
- Payment recording UI (tap to record, auto-link)
- Dashboard summary (key numbers at a glance)

---

## 3. MULTI-TENANCY ARCHITECTURE

**Design Principle:** Isolation-first; one database per tenant (or separate schema) with shared compute.

**Implementation:**
- **Auth:** Tenant ID resolved via subdomain (acme.bms.local) or slug in URL (bms.local/acme)
- **Data Filtering:** Row-level security; every query filtered by tenant_id
- **Branding:** Tenant logo, colors, business name, currency per tenant
- **User Roles:** Admin, Manager, Accountant, Viewer (simple 4-tier)
- **User Invitations:** Admin invites users; email-based signup
- **Data Export:** Tenant can export all data (CSV, JSON)
- **Trial/Billing:** Subscription model (basic, pro, enterprise) — payment integration via Stripe (optional for MVP)

---

## 4. TECHNOLOGY STACK

### Frontend
- **Framework:** Next.js 14+ (App Router)
- **UI Library:** shadcn/ui + Tailwind CSS
- **State Management:** TanStack Query (React Query) + Zustand
- **Forms:** React Hook Form + Zod validation
- **Charts:** Recharts or Chart.js
- **PDF Generation:** pdfkit or react-pdf
- **Barcode/QR:** html5-qrcode (optional MVP)

### Backend
- **Runtime:** Node.js with Express or Next.js API Routes
- **Database:** PostgreSQL (multi-tenant schema approach)
- **ORM:** Prisma
- **Auth:** NextAuth.js or Auth0 (JWT-based)
- **File Storage:** Cloudinary or AWS S3 (for images, receipts)
- **Email:** SendGrid or Mailgun
- **Queue:** Bull (Redis) for async tasks like email, PDF generation

### DevOps
- **Hosting:** Vercel (frontend + serverless API) or Railway/Render
- **Database:** Railway, Neon, or managed PostgreSQL (AWS RDS, DigitalOcean)
- **CDN:** Vercel's built-in or Cloudflare
- **Environment:** Development, Staging, Production

---

## 5. DATA MODEL (Simplified)

```
TENANT
├── Business (logo, name, currency, timezone)
├── Users (email, role, permissions)
├── STOCK
│   ├── Product (sku, name, price, category, reorder_level)
│   ├── Location (warehouse, store, office)
│   ├── StockLevel (product_id, location_id, qty)
│   └── StockMovement (from_location, to_location, qty, type, date)
├── CUSTOMERS
│   ├── Customer (name, email, phone, address, credit_limit, credit_balance)
│   └── CustomerTransaction (invoice_id, payment_id, amount)
├── VENDORS
│   ├── Vendor (name, email, phone, address, payment_terms)
│   └── VendorTransaction (po_id, payment_id, amount)
└── FINANCE
    ├── Invoice (customer_id, total, status, due_date, created_at)
    ├── InvoiceLineItem (product_id, qty, unit_price, amount)
    ├── PurchaseOrder (vendor_id, total, status, due_date, created_at)
    ├── POLineItem (product_id, qty, unit_price, amount)
    ├── Payment (source_id, amount, method, date, status)
    ├── Expense (category, amount, date, description)
    └── AccountingEntry (account, debit/credit, amount, date, ref)
```

---

## 6. USER FLOWS (MVP HAPPY PATHS)

### Flow 1: Add a Product to Stock
1. Admin opens **Stock > Products > Add New**
2. Fills form: SKU, Name, Category, Unit Price, Reorder Level
3. Optionally uploads image
4. Saves → Product created, visible in all locations
5. Stock level defaults to 0 (manual adjustment needed)

### Flow 2: Receive Goods from Vendor
1. Admin opens **Stock > Stock Movements > Receive Goods**
2. Selects vendor, location, picks products from dropdown
3. Enters qty per product
4. Saves → StockMovement created, stock levels updated
5. Optional: Link to PO (if exists)

### Flow 3: Create & Send Invoice
1. User opens **Finance > Invoices > New**
2. Selects customer
3. Adds line items (pick product + qty OR manual amount)
4. System calculates subtotal, tax (if enabled), total
5. Sets due date, payment terms
6. Saves as Draft
7. Reviews invoice
8. Clicks "Send" → Email to customer + Status = "Sent"
9. Payment auto-links when received

### Flow 4: Record Payment
1. User opens **Finance > Payments > Record Payment**
2. Selects payment method (Cash, Mobile Money, Bank)
3. Enters amount, date
4. System shows outstanding invoices, allows quick-select
5. Saves → Payment recorded, invoice status updated to "Partially Paid" or "Paid"

### Flow 5: Dashboard Review (Daily)
1. Owner opens **Home > Dashboard**
2. Sees: Revenue (MTD), Expenses (MTD), Cash Balance, Outstanding Invoices, Low Stock Items, Top Customers
3. Can drill into any widget for details

---

## 7. MVP SCOPE (Phase 1)

### Included:
- ✅ Multi-tenancy (basic: signup, org creation, user roles)
- ✅ Stock: Add products, stock adjustment, stock transfer, low stock alerts
- ✅ Customers: Add, list, view transaction history, credit tracking
- ✅ Vendors: Add, list, view transaction history, payment tracking
- ✅ Invoicing: Create, send (email), track status, record payments
- ✅ Expenses: Log, categorize, monthly view
- ✅ Dashboard: Key financial metrics, stock health, customer insights
- ✅ Mobile-Friendly: Responsive UI, touch-optimized forms
- ✅ Reports: P&L, Receivables Aging, Expense Breakdown, Stock Value (basic CSV export)
- ✅ Multi-currency: Display prices in user's preferred currency
- ✅ User Management: Admin invites users, 3 roles (Admin, Manager, Viewer)

### Excluded (MVP+):
- ❌ Barcode scanning (backend ready, UI deferred)
- ❌ Approval workflows
- ❌ Advanced tax calculation (basic only)
- ❌ Recurring invoices
- ❌ Budget management
- ❌ Multi-language (English only; Kinyarwanda deferred)
- ❌ Mobile app (PWA or web-only for MVP)
- ❌ Payment gateway integration (manual record only)
- ❌ Double-entry accounting (simplified P&L)
- ❌ Batch operations (single record at a time)
- ❌ API for third-party integrations
- ❌ In-app messaging/notifications (email only)

---

## 8. DESIGN LANGUAGE

### Style
- **Color Palette:** Professional blues & greens + neutral grays (Tailwind defaults fine)
- **Typography:** Inter or Poppins (system fonts on mobile)
- **Icons:** Lucide React (32px for mobile touch targets)
- **Spacing:** 8px grid (Tailwind: p-2, p-4, p-6, etc.)
- **Components:** Shadcn/ui (cards, buttons, modals, tables, forms)

### Key Screens
- **Dashboard:** Card-based layout, widgets, trend charts, quick action buttons
- **List Views:** Sortable tables, inline filters, bulk actions toolbar
- **Forms:** Single-column, logical grouping (sections), inline validation, help text
- **Mobile:** Stacked, single-column, hamburger nav, bottom sheet for actions

---

## 9. SUCCESS METRICS (MVP)

- **Performance:** Page load < 2s, mobile-optimized (Lighthouse > 80)
- **Adoption:** 50 test users, 10+ DAU by week 3
- **Usability:** NPS > 50, zero support tickets on core flows
- **Data Integrity:** 100% invoice-payment reconciliation, no lost stock movements
- **Availability:** 99.5% uptime

---

## 10. NICE-TO-HAVES (If time permits)

- Dark mode toggle
- Dashboard customization (drag-to-reorder widgets)
- Inline editing in tables (click to edit, save on blur)
- Quick search (Cmd+K palette)
- Keyboard shortcuts (N = new, E = edit, etc.)
- Notification bell (overdue invoices, low stock, etc.)
- Mobile PWA (offline reading of cached data)
- Kinyarwanda UI strings (future: i18n setup)

---

## 11. ROLLOUT PLAN

| Phase | Focus | Duration |
|-------|-------|----------|
| **MVP (Week 1-2)** | Stock, basic invoicing, customers, vendors, dashboard | 2 weeks |
| **MVP+ (Week 3-4)** | Expenses, advanced reports, mobile polish, user feedback | 2 weeks |
| **Beta (Week 5+)** | Barcode scanning, batch operations, multi-language, scaling | Ongoing |

---

## 12. COMPETITIVE POSITIONING

| Feature | BMS (This) | Wave/Square | Fresh/Zoho | SAP |
|---------|-----------|------------|-----------|-----|
| **Ease of Use** | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐ |
| **Multi-Tenancy** | Native | Multi | Multi | Single |
| **Setup Time** | 5 min | 30 min | 1 hour | Days |
| **Cost** | $9-29/mo | $30-100+ | $20-100+ | $100k+/year |
| **Learning Curve** | Easy | Medium | Medium-Hard | Very Hard |
| **Mobile-First** | Yes | Yes | No | No |
| **Stock + Finance** | ✅ | ✅ | ✅ | ✅ |
| **Customers/Vendors** | ✅ | Limited | ✅ | ✅ |

---

## 13. FUTURE ROADMAP (Beyond MVP)

- **Integrations:** Stripe, M-Pesa, Airtel Money, Pesapal payment gateways
- **Automation:** Recurring invoices, auto-reorder, payment reminders
- **Inventory:** FIFO/LIFO, batch tracking, expiry dates
- **Analytics:** Advanced dashboards, forecasting, trend analysis
- **Social:** Team collaboration, comments, notifications
- **Mobile App:** Native iOS/Android (React Native)
- **Compliance:** Rwanda tax reporting, e-invoice compliance (OBR)
- **Localization:** Kinyarwanda, Swahili, French

---

## 14. ASSUMPTIONS & CONSTRAINTS

- **Users:** Primary: business owners, bookkeepers; Secondary: sales staff, warehouse staff
- **Scale:** Up to 500K products, 100K customers, 1M transactions per tenant (MVP handles smaller; scaling in Q2)
- **Connectivity:** Assumes stable internet; offline mode deferred
- **Browser:** Chrome, Firefox, Safari, Edge (latest 2 versions)
- **Data Residency:** Regional servers (Africa preferred for latency)
- **Compliance:** GDPR-ready (no special handling for Rwanda yet; roadmap item)

---

## 15. KNOWN RISKS & MITIGATION

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Multi-tenant data leakage | Critical | Row-level security testing, audit logs |
| Invoice reconciliation errors | High | Automated matching, manual override + audit trail |
| Stock count discrepancy | High | Adjustment reason tracking, reconciliation reports |
| Tax calculation wrong | Medium | Show disclaimers, allow manual override, consult CPA |
| Mobile UX poor | Medium | Early user testing, responsive design QA |
| Payment integration delay | Low | Defer to MVP+, use manual record for now |

---

## SUMMARY

**Business Management System (BMS)** is a lean, intuitive alternative to heavy ERP suites. It solves the "Big Three" for SMBs: *What do you have?* (Stock), *What do you owe/earn?* (Finance), *Who do you work with?* (Customers & Vendors).

**Target Ship Date:** 2 weeks (MVP), full beta by week 4.  
**Team:** 2-3 engineers + 1 PM/designer.  
**Success = 1,000 signups by month 1, $10K/month ARR by Q2.**
