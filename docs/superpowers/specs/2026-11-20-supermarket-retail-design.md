# Simplex for Supermarkets — Design

**Date:** 2026-11-20
**Status:** awaiting approval
**Baseline:** `main` @ 64d145a (Workshop Steel skin, hardware vertical)

---

## 1. Intent and why

Simplex today is a competent back office with no front of store. It knows products,
stock by location, purchase orders, goods receipts, invoices, payments, customers,
vendors and an audit trail. It has never sold anything to a person standing at a
counter.

The target is Rwandan supermarkets, and the incumbent there is Microinvest — a
Windows desktop suite sold on perpetual licence through local partners. Benchmarking
it (2026-11-20) turned up a specific and exploitable shape:

| Microinvest is strong | Microinvest is absent |
|---|---|
| Touch POS, customer display | No dashboard anywhere in the product |
| Fiscal printers, X/Z reports | Not one named report ("a vast number of reports") |
| Counter scales control | No shrinkage or waste accounting |
| Lots and serial numbers | Web client last shipped **May 2017** (v3.07.035) |
| Replication across stores | No expiry-date stock tracking |

So the win condition is not feature parity. It is: match them at the lane, beat them
everywhere behind it, and be the only credible browser-native option in the market.

**v1 scope decision: win Rwandan supermarket deals.** Not full suite parity, not
till-only.

---

## 2. Goals

1. A compliant checkout lane — scan, weigh, tender, print an EBM-valid receipt, close a shift.
2. RRA certification as a Certified Invoicing System (CIS) integrating via VSDC.
3. Grocery stock reality: expiry dates, batches, waste write-offs, counted stocktake with variance.
4. Extend the back-office lead: shelf-edge reports, basket analysis, shrinkage ledger — the things their brochure cannot name.
5. The Shelf Edge skin, shipped independently and ahead of all of the above.

## 3. Non-goals

- Restaurant mode, table plans, recipes and production. Microinvest has them; they are not supermarket work.
- A barcode **label design studio** (their Barcode Printer Pro). We print shelf labels from fixed templates.
- Serial numbers and warranty cards — electronics retail, not grocery.
- Replication topology. We are multi-tenant cloud; that is the point, not a gap.
- Markets outside Rwanda in v1. See the `FiscalAdapter` seam in §6.4.

## 4. Anti-goals

Things that would make this a failure even if every ticket closed.

- **A lane slower than Microinvest's.** Cashiers measure in seconds per basket, not features. If scan-to-tender is slower than their touch build, nothing else matters.
- **Any path where a sale completes without a compliant receipt.** That is a legal exposure for the shop, not a bug for us.
- **A till shop staff cannot install or recover.** If recovery needs us on the phone, we have shipped a support burden, not a product.
- **Ending up as a second system beside Microinvest.** If shops keep both, we have lost.

---

## 5. Constraints

These are hard, taken from the RRA specifications (`RRA_VSDC_DOC_v1.0.5`, 16 Apr 2022,
and `Technical Specification of CIS for VSDC, Ver.1.0`, March 2018), read 2026-11-20.

**C1 — Offline selling is prohibited.** CIS spec §24.7.9: *"A Certified Invoicing System
must stop operating, whenever a communication error appears, due to the disconnection
from the VSDC or due to power failure interruption. After the error has been cleared,
the continuation of the printing process must be enabled."* An offline-first till is
not a feature to build; it is non-compliance. The lane must hard-stop.

**C2 — The VSDC is local, not cloud.** It ships as a Java **WAR deployed on the
merchant's own webserver** (Tomcat, Jetty or Apache), REST, JSON, Windows 10+/Linux/Unix.
Simplex's servers cannot reach it. Something of ours must run on that counter.

**C3 — Transmission is batched, not real-time.** VSDC→RRA data flow is *"scheduled to
a maximum of 15 min"*. The CIS is responsible for holding the last-received datetime
per data type and passing it on the next pull. Internet loss is therefore survivable
for a window; VSDC loss is not (see C1).

**C4 — MRC format is assigned, not chosen.** For POS systems: `BBBCCNNNNNN` where
`BBB` = software developer id (given by RRA), `CC` = developer certificate number
(given by RRA), `NNNNNN` = serial in ascending order, assigned by us per installation.

**C5 — Receipt layout is mandated.** Every receipt carries an `SDC INFORMATION` block:
Date, Time, SDC ID, RECEIPT NUMBER, Internal Data (dashed every 4 chars), Receipt
Signature (dashed every 4 chars), and a QR code built as
`invoice_date(ddmmyyyy)#time(hhmmss)#sdc_number#sdc_receipt_number#internal_data#receipt_signature`.
Plus CIS-originated receipt number, date/time and MRC. `COPY`, `TRAINING` and
`PRO FORMA` types must be visually distinguishable from `NORMAL`.

**C5b — The code tables are RRA's, not ours.** Tax type is a four-value set:
`A` (A-EX, exempt), `B` (B-18.00%), `C`, `D` — an earlier draft of this document said
A/B only and was wrong. Payment method is likewise an RRA code list, and it already
includes `06 MOBILE MONEY` ("any transaction using mobile money system") alongside
`01 CASH`, `02 CREDIT`, `03 CASH/CREDIT`, `04 BANK CHECK`, `05 DEBIT&CREDIT CARD`.
Recording MoMo as a tender is therefore compliant on its own; integrating with a MoMo
API is a separate product decision, not a compliance one. **Decided:** v1 records the
tender under `06` and does not integrate. The till takes the cashier's confirmation
that money arrived, exactly as it does for cash. Integration is planned, not dropped —
`Tender` carries a nullable `providerRef` and a `confirmed` flag from Phase 1
so that a later MTN MoMo or card-terminal integration is a configuration switch per
org and per lane, not a schema change. Nothing in v1 writes to those fields.

**C6 — Stock movement is a reporting obligation.** Not just invoicing.
`/stock/saveStockItems`, `/stockMaster/saveStockMaster`, `/trnsPurchase/savePurchases`.
The spec is explicit that *"Every stock inventory information must have its stock
in/out information sent in advance"* — ordering matters. Our existing `StockMovement`
and `StockLevel` models map onto this almost directly.

**C7 — Certification is a document exercise as much as a technical one.** RRA requires:
business registration certificate, valid RSSB clearance, valid RRA tax clearance,
product brochure, product warranty statement, physical address, **user manual**,
**installation guide**, and **programming and configuration manual**, emailed to
`cis_sdc_certification@rra.gov.rw`. Cost and turnaround are not published.

**C9 — The till runs on whatever Windows the shop already owns.** Microinvest is a
Windows desktop product, so every machine we displace is a Windows PC of unknown
vintage. Tauri 2 requires Windows 10 1803 or later plus the Edge WebView2 runtime
(pre-installed on Windows 11 and on Windows 10 since the Evergreen rollout, absent on
Windows 7 and 8.1). This is the one assumption in the whole design that cannot be
checked from here — see §9 Q6. If pilot hardware is older than that, §6.4's choice of
Tauri flips to a pure-Rust UI and the Shelf Edge skin is rebuilt rather than reused.

**C8 — Unknowns to resolve before building against them.** Whether a SaaS-hosted CIS
is acceptable at all is *not* stated publicly; RRA's own "Online EBM" is capped at
≤10 invoices/month, which excludes supermarkets, while VSDC is offered for "own
invoicing system" with no stated platform constraint. Sandbox availability,
certification cost and timeline are also unpublished. See §9.

---

## 6. Approaches

### 6.1 What runs at the till

| | Approach | Verdict |
|---|---|---|
| A | **Browser-direct to localhost.** POS page fetches `http://localhost:8080/...` and drives hardware over WebUSB/WebSerial/WebHID. | **Rejected.** Safari blocks localhost calls from an https page; WebSerial is Chrome-only. Too fragile for a legally-mandated receipt path, and there is nowhere to hold a queue when the tab closes. |
| B | **Split: a local agent plus a cloud-served lane.** Installed service owning VSDC calls and hardware; the POS itself a PWA from our servers talking to it over `localhost:7331`. | **Rejected on revision.** See below. |
| C | **One installed desktop application** that is both the lane and the local agent. | **Chosen.** |

**Chosen C; B was chosen first and is recorded here because the reasoning for C
depends on it.** B's premise was that browser-native delivery is Simplex's advantage
over Microinvest. At the lane that premise does not hold. C1 already tethers the till
to a local component, so the lane is never purely browser-delivered; B therefore paid
for a second component, a `localhost` transport, a pairing token, an origin-check
story, and a service worker whose only job was to make §7's "trades through a cloud
outage" claim true — all to preserve a property the regulation had already taken away.

C deletes every one of those. One installer, one process, one update path. Offline at
the lane stops being a cached fallback and becomes the resting state. The `invcNo`
sequence lives in the application's own local database, where it is gapless and local
by construction rather than by discipline. And it is the artifact RRA certification is
shaped around: they ask for an installation guide and a programming and configuration
manual, which describe an installed program, not a web page.

Browser-native delivery remains Simplex's advantage over Microinvest — in the **back
office**, where it is actually true and where Microinvest's own web client has not
shipped since May 2017. §6.6 keeps it there.

Cost accepted: we ship, sign, support and update a desktop binary, and the till UI is
no longer reachable by simply opening a URL.

### 6.2 v1 scope

| | Approach | Verdict |
|---|---|---|
| 1 | **Win Rwandan supermarket deals** — till + EBM + grocery essentials, beat them on back office. | **Chosen.** |
| 2 | **Full suite parity** — restaurant, recipes, label design, replication, warranty cards. | Rejected. Most of it is not supermarket work. |
| 3 | **Till only** — add a lane, shops keep Microinvest for the rest. | Rejected. Guarantees we stay the second system. |

### 6.3 Skin

Direction **A, "Shelf Edge"**, chosen from three presented. **Workshop Steel is
retired on `main`**, confirmed by the user, exactly as Workshop Steel retired Atelier.
Simplex carries one skin at a time; the two live orgs move with it. Price-label motif: white
card, heavy black numerals, one hi-vis accent, department colour chips. Chosen over
"Cold Chain" (better story, but every figure in it needs expiry and waste data that
does not exist yet) and "Market Hall" (too close to the retired Atelier palette to
read as a new vertical).

### 6.4 Implementation language for the till

| | Approach | Verdict |
|---|---|---|
| 1 | **Rewrite everything in Rust** — `axum` + `sqlx` + Postgres for the server, Rust UI throughout, no TypeScript anywhere. | **Rejected.** 13,356 lines, 51 API routes and 24 Prisma models already work. Re-deriving auth, RBAC, the audit log, soft deletes, pagination, the admin console with org impersonation, CSV import and every report produces a system that does what today's does, spends the whole v1 budget doing it, and requires migrating two live orgs onto a server that does not exist yet. No shop owner can perceive the difference. |
| 2 | **Rust till, existing Next.js back office.** | **Chosen.** |
| 3 | **TypeScript till** — Electron shell, or the §6.1-B service worker. | **Rejected.** Electron drags a browser and a Node runtime onto every lane for roughly 400 MB to solve a problem a 10 MB binary solves. |

**Why 2.** The till does not exist in the repository yet, so there is nothing to throw
away by writing it in Rust — and Rust fits this particular component better than the
alternatives: a serial scale, raw ESC/POS bytes, a drawer kick, a local SQLite store,
an outbound queue that must not lose a sale, and a process running unattended for a
fourteen-hour trading day. It ships as a single `.msi` with no runtime to provision per
lane, which is the practical difference between an install a shop's own IT can perform
and one that needs us on site.

What Rust is **not** chosen for: speed at the lane. Scan-to-tender is dominated by UI
paint and the VSDC round trip, not by language, and the Phase 2 speed gate must not be
mistaken for something the language delivers on its own.

Cost accepted: fewer people locally can maintain Rust than TypeScript. Confining it to
the till holds that exposure to one component instead of the product.

**Shell: Tauri 2** — Rust core, React and Tailwind in the system webview. This keeps
one design system across till and back office, so Shelf Edge is built once and the
lane screen can be built by whoever builds the rest of the UI. The alternative
(`egui`, `Slint` — pure Rust, own renderer, no webview) wins only on old Windows and
costs a rebuilt skin and a UI only a Rust developer can touch. The deciding fact is
C9, which we cannot check from here; the choice is therefore revisited once, against
real pilot hardware, before the lane is built.

### 6.5 Fiscal layer shape — my call, not asked

EBM goes behind a thin `FiscalAdapter` interface with exactly one implementation
(`EbmRwandaAdapter`). The seam costs close to nothing now and is the difference
between a port and a rewrite if a second market ever appears. No second adapter gets
built in v1, and no abstraction is added beyond the interface itself.

The adapter now lives in the Rust crate rather than a Node service; the seam itself
is unchanged.

### 6.6 What stays in the cloud

Catalogue, stock truth, pricing, promotions, purchasing, every report, the admin
console and the audit log stay in Next.js and Postgres, unchanged and in TypeScript.
This is deliberate: it is the half of the product Microinvest cannot match, and it is
the half where a browser really is the right delivery mechanism.

---

## 7. Architecture

```
  ┌───────────────────────────────┐
  │  Simplex cloud                │  catalogue, stock truth, pricing,
  │  Next.js + Postgres           │  promotions, purchasing, reports,
  │  (browser, Shelf Edge skin)   │  admin console, audit log
  └───────────────┬───────────────┘
                  ▲  https, when available
                  │
  ═══════════ the till PC ═════════════════════════════
                  │
  ┌───────────────┴───────────────┐
  │  SIMPLEX TILL  (Tauri 2)      │  <- new; we ship, sign and certify this
  │                               │
  │  React + Tailwind in webview  │  the lane UI, Shelf Edge
  │  ---------------------------  │
  │  Rust core:                   │
  │   - basket, pricing, tender   │
  │   - SQLite: catalogue cache,  │
  │     sales, gapless invcNo,    │
  │     outbound queue            │
  │   - VSDC REST client          │
  │   - ESC/POS printer           │
  │   - scale (serial)            │
  │   - scanner (HID wedge)       │
  │   - drawer kick               │
  └───────────────┬───────────────┘
                  │  http, localhost:8080 or shop LAN
  ┌───────────────▼───────────────┐
  │  RRA VSDC (.war on Tomcat)    │ ──► RRA EBM 2.1 API  (<=15 min batches)
  └───────────────────────────────┘
```

**One process, not two.** The lane UI and the local agent are the same application.
There is no `localhost` hop between them, no pairing token, no origin-check surface
and no service worker. §6.1 records why this replaced the split design.

**VSDC topology is configurable.** The diagram allows either. One WAR per till and one
WAR per branch over the shop LAN are both plausible and RRA does not say which is
expected, so the till takes a VSDC base URL as configuration, defaulting to
`localhost:8080` and accepting a LAN host. The choice changes the blast radius of C1 —
a per-branch VSDC means one failure stops every lane — so it is also §9 Q5.

**Division of responsibility.** The cloud owns the catalogue, stock truth, pricing,
promotions and all reporting. The till owns everything that must not depend on the
internet at the moment of sale: the basket, the VSDC conversation, the printer, the
scale, the drawer, and the queue of sales not yet acknowledged by the cloud.

**The sale, end to end.**

1. Cashier scans or keys a line. Price and tax class come from the till's own SQLite catalogue cache, refreshed from the cloud at shift open and in the background, so a scan never waits on a round trip.
2. Weighed item: the Rust core reads the scale, or the line is decoded from a weight-embedded barcode.
3. Tender. Cash, card, mobile money, voucher or mixed, recorded against the C5b code list. Per C5b, v1 records the tender; it does not confirm payment with a provider.
4. Till allocates `invcNo` from a **gapless per-`bhfId` sequence in its own SQLite**. This cannot come from the cloud: the sequence is consumed before the cloud is touched, and the cloud may be unreachable. The existing per-org `InvoiceSequence` therefore does not serve the lane.
5. Till calls `POST /trnsSales/saveSales` on the VSDC with `tin`, `bhfId`, `invcNo`, `salesTyCd`, `rcptTyCd`, `pmtTyCd`, `salesSttsCd`, `cfmDt`, `salesDt`, `totItemCnt`, `taxblAmtA/B`, `taxAmtA/B`, `totTaxblAmt`, `totTaxAmt`, `totAmt`, and a line array of `itemSeq`, `itemCd`, `itemClsCd`, `itemNm`, `qty`, `prc`, `splyAmt`, `dcRt`, `dcAmt`, `taxTyCd`.
6. VSDC returns `rcptNo`, `intrlData`, `rcptSign`, `totRcptNo`, `vsdcRcptPbctDate`, `sdcId`, `mrcNo`.
7. Till composes the receipt per C5 and prints. **Only now** is the sale complete.
8. The sale is written to local SQLite and queued for the cloud, for stock, reporting and audit. This step may lag indefinitely; steps 5-7 may not.
9. Stock in/out goes to the VSDC per C6, in the mandated order.

Steps 4 through 7 happen entirely inside one process on one machine, which is the
whole point of §6.1-C.

**Failure behaviour, by C1.** VSDC unreachable → the lane refuses new sales and says
so plainly, naming the fault and what to do. It does not queue sales and pretend.
Cloud unreachable → the lane keeps trading on its local catalogue and grows its
outbound queue; this is permitted because the VSDC, not our server, is the legal
counterparty, and it needs no special mechanism, because local is already where the
till reads and writes.

---

## 8. What gets built

Phased so each phase is independently shippable and the compliance risk sits early.

### Phase 0 — Shelf Edge skin — **COMPLETE (2026-11-20)**

Token block in `src/app/globals.css`, font pair in `src/app/layout.tsx`, and the
trade vocabulary across the nine files that carry it. Replaces Workshop Steel on
`main`, as Workshop Steel replaced Atelier — approved by the user, not assumed.
This phase is pure Next.js and is unaffected by §6.4; it ships before any Rust exists,
and the same tokens are later consumed by the till's webview.

### Phase 1 — Grocery and retail data model

`Product`: barcode (EAN-13/8), PLU, `isWeighed`, `taxTyCd` (A/B/C/D per C5b),
department, pack size, shelf-life days.

New grocery models: `Batch` (lot, received, best-before), `StockWriteOff` (reason:
expiry, damage, theft, markdown), `Promotion`, `PriceChange`. `StockMovement` grows a
`batchId` and an EBM sync state.

New retail models, which the first draft of this document omitted: `Lane`, `Shift`,
`Sale`, `SaleLine`, `Tender`. `Tender` carries `pmtTyCd`, amount, and — per C5b — a
nullable `providerRef` and a `confirmed` boolean that v1 never writes, so that payment
integration later is configuration rather than a migration. **Till sales do not reuse `Invoice`.** An invoice carries
credit terms, a required customer and a payment lifecycle; a till sale is settled at
the moment it exists. Forcing one into the other would corrupt both — and `Invoice`
already serves the B2B side of these shops unchanged.

**`qty` becomes decimal.** Eight columns are currently `Int`
(`StockLevel.qty`, `StockMovement.qty`, `InvoiceLineItem.qty`, `SalesOrderLine.qty`,
`DeliveryNoteLine.qty`, `PurchaseOrderLine.qty`, `GoodsReceiptLine.qty`, plus
`Product.reorderLevel`). Weighed goods make that wrong — 0.734 kg of tomatoes is not
an integer. This migration touches every stock path in the app and is the single
riskiest change in Phase 1, so it goes first, alone, with its own backfill.

Plus a real grocery seed: a Rwandan supermarket's actual department spread, not 47 widgets.

### Phase 2 — The till

One Tauri 2 application, built in this order so the install and support story is proven
before compliance is on the line.

**2a — Shell and install.** Tauri project, Rust workspace, signed `.msi`, update
channel, and a lane-to-org pairing that registers the install against a `Lane`.
Ships with the installation guide C7 demands — written now, not at certification.
**First action of this phase is to verify C9 against real pilot hardware**; if it fails,
§6.4's Tauri choice flips to a pure-Rust UI before any screen is built.

**2b — Hardware.** ESC/POS printing, barcode scanner (HID keyboard-wedge first, it is
what shops already own), serial scale, drawer kick. Each behind a Rust trait with a
simulated implementation, so the lane is testable in CI without a counter.

**2c — Local store.** SQLite via `sqlx`: catalogue cache, `Sale`/`SaleLine`/`Tender`
mirrors, the gapless per-`bhfId` `invcNo` sequence, and the outbound queue. The sale
and its queue entry are written in one transaction.

**2d — The lane.** Full-screen POS: scan, weighed items, price lookup, line void,
basket discount, suspend/resume, mixed tender (per the C5b code list), refund against
original receipt, buyer TIN capture for B2B sales so the customer can claim input VAT,
shift open and close, cash declaration, X and Z reports. Shelf Edge, built for speed
and a touch screen.

Basket, pricing and tender logic lives in the **Rust core**, not the webview. The
earlier design put it in a framework-free TypeScript module so it could migrate into
the agent if §9 Q1 went against us; with §6.1-C that migration has already happened
and the hedge is retired.

**The gate: scan-to-tender must beat Microinvest's touch build on the same hardware.**
That is the anti-goal in §4 made measurable, and it gets benchmarked here, not later.
It needs the pilot shop and a running Microinvest — see §9 Q7.

### Phase 3 — EBM integration

`FiscalAdapter` + `EbmRwandaAdapter` in the till's Rust core. Device initialisation, code and
item-classification sync, item registration, sales transactions, purchase
transactions, stock in/out per C6. Compliant receipt composition and QR per C5. The
hard-stop of C1. Built against Test VSDC throughout.

### Phase 4 — Certification

MRC assignment once RRA issues `BBB` and `CC`. The nine documents of C7, including
the programming and configuration manual. Submission to
`cis_sdc_certification@rra.gov.rw`, then whatever they come back with.
**This phase has an unknown duration and is not ours to control.**

### Phase 5 — The back-office lead

The part that actually wins deals, built on data the earlier phases start producing:
shrinkage ledger and waste reporting, expiry and FEFO alerts, basket analysis,
sales-per-hour and per-cashier, margin by department, counted stocktake with
variance, automated reorder from reorder point and sales velocity, shelf-edge label
printing. Every one of these is something Microinvest's own marketing cannot name.

---

## 9. Open questions

Q1–Q5 are unanswerable from public sources and must go to RRA directly. Phases 0–2 are
not blocked by them; Phase 3 is. Q6-Q8 are ours to find out; Q6 and Q7 land on the
pilot shop.

1. **Is a SaaS-hosted CIS certifiable at all?** Still worth asking, but **§6.1-C has largely de-risked it.** Under the chosen design the invoicing system *is* a locally installed program that talks to a local VSDC, which is what the spec appears to assume throughout; the cloud holds the catalogue and the reports, not the invoicing. A "no" from RRA now costs a configuration change, not a redesign.
2. **Is there a sandbox / Test VSDC we can get before certification?** §1.2 names a Test VSDC and a test API server; access terms are unstated. Phase 3 is near-unbuildable without it.
3. **Certification cost and turnaround.** Unpublished. It sits on the critical path to revenue.
4. **Does a multi-tenant cloud serving many shops need one MRC per shop, per lane, or per software instance?** C4 says serial "in ascending order, given by the software developer" — the unit is not specified, and it determines whether MRC is a column on `Org`, on a `Lane`, or on the till install.

5. **Is the VSDC expected per till or per branch?** Not stated. It decides whether a single VSDC failure stops one lane or the whole shop, and therefore how much redundancy the till needs.

6. **What Windows version and WebView2 state do real pilot tills have?** C9. Decides Tauri versus a pure-Rust UI, and therefore whether Shelf Edge is reused or rebuilt at the lane. First action of Phase 2a.

7. **Is there a pilot shop, and can we get at a running Microinvest?** The Phase 2 speed gate has no baseline without both, which makes the sharpest quality bar in this document unmeasurable. Still open.

8. **Where does the company paperwork stand?** C7 wants business registration, RSSB clearance and RRA tax clearance, and the software developer id `BBB` must be issued before any MRC exists. Still open; if none of it has started, Phase 4 is longer than the build.

Action: email `cis_sdc_certification@rra.gov.rw` during Phase 0 so answers arrive
before Phase 3 starts. Recommend also pulling the `.xlsb` checkpoint workbook RRA
publishes — its "Main" sheet is the certification pass/fail list and may answer (1)
without a conversation.

---

## 10. Left to the builder

- Rust crate layout inside the Tauri workspace, and where the IPC boundary between webview and core is drawn.
- Local catalogue cache mechanics: schema, refresh trigger, invalidation, and what happens when the cloud's catalogue has moved on mid-shift.
- Outbound queue semantics: retry policy, ordering guarantees, and how a sale the cloud has already accepted is recognised after a crash.
- Code sharing between the till webview and the Next.js app: whether the Shelf Edge tokens and shared components come from a workspace package or are duplicated. Duplicated is acceptable in v1.
- Installer signing: certificate, and whether updates are self-hosted or go through a provider.
- POS keyboard map and touch target sizing, subject to the Phase 2 speed gate.
- Which specific ESC/POS printers and scales to certify against first. Pick what Kigali supermarkets already own; find out before choosing.
- Test strategy per phase, including how the VSDC is faked for CI.
- Whether `Batch` is required for all products or only those with `shelfLifeDays` set.

**Promotion mechanics, settled rather than left open:** v1 does time-boxed price
overrides plus percentage or amount discounts at line and basket level. No multi-buy,
mix-and-match or loyalty-conditional pricing engine. Those are Phase 5 at the earliest,
and Microinvest does not describe a mechanism for them either.

---

## 11. Sources

- `RRA_VSDC_DOC_v1.0.5` — RRA VSDC API Documentation, 16 April 2022 (doc rev 1.0.5, 16/05/2023).
- `Technical Specification of CIS for VSDC, Ver.1.0` — RRA, March 2018.
- RRA, "Available EBM systems and concerned taxpayers", rra.gov.rw, read 2026-11-20.
- Microinvest English product pages (retail-supermarket, POS, Warehouse Pro, Warehouse Pro Web, Barcode Printer Pro, Visual Styles), read 2026-11-20. Marketing copy, not documentation.
- Simplex `main` @ 64d145a and the running application.
