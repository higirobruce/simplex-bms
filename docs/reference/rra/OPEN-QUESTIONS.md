# RRA questions — open, resolved, and how to settle them

Questions about the Rwanda Revenue Authority's specifications that the vendored
documents in this directory do not answer, or answer only by inference.

Each entry says what we assume today, what it costs if the assumption is wrong,
and how to settle it. **Do not change an assumption without recording the
evidence here**, because several of them are load-bearing and the cost of being
wrong is usually a burned invoice number.

The authorities are `vsdc.txt` (VSDC specification 1.0.4) and `cis.txt`
(Technical Specification of CIS for VSDC, Ver.1.0), both in this directory.

---

## Resolved

### `lastXxxNo` means the LAST number used, not the next to issue

**Resolved 2026-11-21 from the specification itself. No change required —
the code already allocates `last + 1`.**

This was the sharpest open risk in the project: an off-by-one here makes every
reinstalled till either skip a number or reuse one, and reuse produces an RRA
`994` on the first sale, which is unrecoverable because a 994 carries no receipt.

Three pieces of evidence, all in `vsdc.txt`:

1. **RRA's own attribute names say "Last".** The `InitDevice` table
   (`vsdc.txt:514-531`) names them *Last Sale Invoice Number*, *Last CIS invoice
   number*, *Last Copy Invoice Number*. Not "next".

2. **A fresh device returns `0`.** The specification's own JSON response sample
   (`vsdc.txt:536-538`) shows:

   ```
   "lastPchsInvcNo": 0,  "lastSaleRcptNo": 0,  "lastSaleInvcNo": 0,
   "lastInvcNo": null,   "lastTrainInvcNo": null,  "lastCopyInvcNo": null
   ```

   **There is no invoice number 0.** Under a "next to issue" reading a fresh
   device would have to return `1`, because 1 is the first number it would ever
   issue, and `0` would be meaningless. Under "last used", `0` reads naturally as
   "none issued yet".

3. **A sale sample carries `"invcNo": 1`** — which is exactly `0 + 1`.

The same 0-versus-null split was already the basis for choosing `SaleInvc` as a
sale's counter (see below), so the two readings are consistent with each other.

**Still worth confirming empirically** when a Test VSDC is available, because
inference from a sample is not the same as observed behaviour. See *The four-step
probe* at the end.

---

## Open — ordered by what they cost if we are wrong

### 1. What does a `null` counter become? (Phase 3, highest risk)

Four of the seven counters come back **`null`** on a fresh device, not `0`.
`Store::reconcile_counter_from_vsdc` takes `last_invc_no: i64`, so the Phase 3
client must map `null` to something.

- **`null` → 0 is correct.** The receipt type has never been used, so the first
  one issued is 1.
- **`null` → 1 reintroduces the off-by-one** resolved above. The first Training
  or Copy receipt then collides and returns a 994 that cannot be recovered.
- **`null` → error is also wrong.** A brand-new till returns `null` for most of
  its counters, so it could never be reconciled at all.

This is now the sharpest remaining risk in the invoice-numbering path. It is
narrower and cheaper to get right than the question it replaced, but only if
Phase 3 meets it as a decision rather than a surprise.

### 2. Which of the seven counters does a normal sale draw from?

We use **`SaleInvc`**. The specification never states it. The reasoning: a fresh
device initialises `lastSaleInvcNo`, `lastSaleRcptNo` and `lastPchsInvcNo` to
`0` while the other four come back `null`, so the three at zero are the ones in
use; and `lastInvcNo` ("Last CIS invoice number") is more plausibly the home of
C5's separately mandated CIS-originated receipt number.

`NewSale.counter` is an explicit argument rather than a hard-coded value, so the
cost of being wrong is one call site.

### 3. The response shape contradicts itself

`vsdc.txt`'s attribute table nests the counters under `InitDevice`, while its own
JSON sample puts them **flat** under `data.info`. The flat sample is the more
likely wire reality, so the Phase 3 parser must accept it. Assuming the nested
shape would fail on first contact with a real device.

### 4. Reconciling upward while a sale is pending below

If the VSDC reports having seen up to N and a local sale is still `AwaitingVsdc`
holding M < N, then the VSDC *does* hold that invoice, so the sale should be
resolved rather than left pending. This is untested behaviour and belongs with
the Phase 3 operator-reconciliation path.

### 5. Receipt formatting — four smaller questions

These change what prints, not what is recorded, so each costs a formatting change:

- **Money:** does the printed receipt require two decimals, as the samples show
  (`1000.00`), or are whole francs acceptable? RWF has no circulating subunit, and
  the VSDC receives exact decimal strings in the payload regardless.
- **Quantity:** `cis.txt:298` shows a quantity as `1.00` — two decimals. The screen
  shows a whole quantity as `2` and a weighed one as `1.250`.
- **QR `sdc_receipt_number`:** is it `rcptNo` or `totRcptNo`? We use `rcptNo`.
- **QR internal data and signature:** dashed every four characters as they print,
  or raw? We send them raw.

### 6. Tax bands C and D

The specification's samples show only `A-EX` and `B-18.00%`. `till-core` returns a
zero rate for C and D, and the receipt renders `C-0.00%` as a guess. What rates and
labels do they actually carry?

### 7. The embedded-price digit layout on scale labels

Real scales differ in how they pack weight or price into an in-store EAN-13. Our
decoder takes the layout from configuration (`EmbeddedMode`), but the pilot shop's
actual scales decide the correct setting.

---

## The four-step probe

Several of these settle in one short session with a Test VSDC:

1. Initialise a fresh device and record all seven counters, noting which are `0`
   and which are `null`.
2. Send one normal sale.
3. Read the counters again.
4. Compare.

If `lastSaleInvcNo` moves `0 → 1`, that confirms both the last-used reading and
`SaleInvc` as the sale's counter. Whichever counter moves answers question 2
directly. The response shape answers question 3 on the first call.

---

## Related carried requirements (not questions for RRA)

- **The installer must persist and recover `Lane.dvcSrlNo` across a reinstall.**
  `reconcile_counter_from_vsdc` is keyed on the device serial. If a rebuilt till
  registers as a new device, it starts a fresh counter at 1 and the old numbers are
  orphaned at the VSDC. This is a hard requirement on the Phase 2a installer.
- **A reprinted COPY currently carries the original's receipt counter.** §7.25.1
  wants a per-receipt-type series. Closing it needs a VSDC round-trip and
  `Counter::CopyInvc`. Documented in `till/crates/till-lane/src/receipt.rs`.
