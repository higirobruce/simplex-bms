# Till UI Toolkit — Design (short)

**Amends** `2026-11-20-supermarket-retail-design.md` §6.4 (implementation language
for the till) and C9 (unknown Windows vintage). Decided 2026-11-21.

## Intent and why

C9 cannot be answered. §9 Q7 is still open — **there is no pilot shop**, so no one
can read an OS build number off a till we are going to displace. The spec made
"verify C9 against real pilot hardware" the first action of Phase 2a, which means
Phase 2 is parked indefinitely on a fact nobody can obtain.

Rather than carry an unquantified WebView2 risk into 2d — the largest and most
expensive phase to redo — we remove the dependency. **The lane is a pure-Rust
desktop application.** C9 then leaves the critical path entirely, and 2a and 2d
unblock today.

## Goals

- A desktop app with no webview runtime and no bundled browser.
- Installs as effectively one executable, on whatever Windows the shop owns.
- Shelf Edge reproduced at the lane, not approximated.
- Scan-to-tender fast enough to beat a native competitor.

## Non-goals

- A browser-based or cloud-hosted till. Ruled out on C2, not on taste: the VSDC is
  RRA's Java WAR on the counter, and it will never return an
  `Access-Control-Allow-Origin` for our origin. Any fix requires a process on the
  counter, so the local process is the architecture, whatever renders the UI.
- Reusing the Phase 0/1 React components at the lane. They stay for the back office.
- Mobile or tablet lanes. MoMo API integration (C5b: recorded as tender `06`, not
  integrated).

## Anti-goals

- Shipping a UI whose viability depends on a fact nobody can check.
- A lane slower than Microinvest's touch build. That is the §4 anti-goal, and a
  webview start-up cost is a standing tax against it.

## Constraints

- **C1** the lane hard-stops on VSDC communication failure; offline selling is
  prohibited.
- **C2** the VSDC is local. Something of ours runs on that counter.
- **C5 / C5b** mandated receipt layout, SDC block and QR; RRA's own code tables.
- **C9** neutralised by this decision rather than answered.
- **§4 speed gate unmeasurable.** Beating Microinvest needs a running Microinvest
  and a pilot shop (§9 Q7). Neither exists. Replaced by an internal latency budget
  we can measure on any machine — *my proposal, not an instruction from the spec*:
  keystroke to line on screen < 50 ms; scan to line committed < 100 ms; tender to
  receipt bytes leaving the process < 300 ms; cold start < 2 s. Instrumented from
  the first screen, so the real gate can be checked the day a pilot appears.

## Approaches

**1. Tauri 2 + the existing Shelf Edge React.** Reuses the skin and the team's
existing idiom. Costs a WebView2 dependency and a Windows 10 floor we cannot
verify, plus an IPC boundary to design between webview and core (§10). Rejected:
the risk is unquantifiable with no pilot shop, and it lands on the phase that is
most expensive to rebuild.

**2. A local Rust agent serving a web UI to the browser already on the machine.**
Kills C9 outright, reuses the skin as-is, smaller install than Tauri, and keeps the
"locally installed invoicing system" story §9 Q1 depends on. Rejected by the
decision to ship desktop apps: a browser is weakly locked down (F11, tabs,
navigating away) and a `127.0.0.1` endpoint is a security surface needing a token
and strict CORS.

**3. Pure-Rust desktop UI — chosen.** No runtime, no webview, works below the
Windows 10 floor, one language shared with the already-merged `till-core` and
`till-store`, and no IPC seam at all — which deletes an item §10 left to the
builder. Costs a skin rebuild and hand-built layout instead of CSS.

The original argument for affordability was that Shelf Edge was flat (2px corners,
no shadows). The skin changed the next day (below), so that argument was re-checked
rather than inherited: in `egui`, rounded rectangles and 1px strokes cost the same
as square ones, and fonts embed as TTFs. The one thing that does not port cheaply is
a shadow-heavy, layered skin — so Cloud & Azure carries its depth in hairlines and a
tinted second surface rather than drop shadows. That is both cheaper to draw and
closer to the references it came from.

## The how, as decided

- **egui + eframe.** Fonts embedded as TTFs (Archivo, IBM Plex Mono). Tokens ported
  to Rust constants from `src/app/globals.css`, so the two surfaces cannot drift
  silently.
- **Why not the alternatives:** `iced` is a reasonable retained-mode fallback if
  hand-built layout proves painful. `Slint` is technically well suited to kiosk and
  touch but is licence-encumbered for a commercial product (GPL or paid) and so
  needs a deliberate commercial decision — flagged, not chosen. `Dioxus` desktop
  renders in a webview, reintroducing exactly what this decision removes.
- **Phase 2b hardware stays generic.** §10 asks which ESC/POS printers and scales to
  certify against first, and says to find out before choosing. With no pilot shop
  there is nothing to find out, so: traits with simulated implementations, written
  against a generic ESC/POS profile, with no model-specific quirks until a real
  shop exists. This is what the spec already prescribed for 2b; the lack of a pilot
  settles the open item rather than blocking it.

## Skin — Cloud & Azure

Chosen 2026-11-21 from three directions, against three references the user supplied
(PosFood, PikkoPay, FastCart). Supersedes Shelf Edge, including in the back office:
Simplex carries one skin at a time.

The shared grammar of all three references, and now ours: a near-white ground, white
surfaces, hairline borders instead of heavy rules, generous rounding, muted grey for
everything secondary, and exactly one saturated colour.

| Token | Value | Role |
|---|---|---|
| `--bg` | `#F1F6FB` | cool near-white canvas |
| `--surface` | `#FFFFFF` | panels |
| `--ink` | `#0F1B2A` | deep navy, not neutral black |
| `--ink-mute` | `#546A7E` | secondary text, AA on every ground |
| `--line` | `#E2EBF4` | the hairline the whole skin leans on |
| `--accent` | `#176596` | fills carrying white text at body size |
| `--hi-vis` | `#3FA9E5` | bright azure: a GROUND for dark text, never text |
| `--radius` | `14px` | generous rounding |

**Two azures, deliberately.** `--accent` is dark enough to carry white text at body
size (AA); `--hi-vis` is the bright brand blue and fails against white, so it is only
ever a ground for dark text. Swapping them is the one way to break this palette's
contrast guarantees, and `tests/skin/tokens.test.ts` asserts both contracts by name.

**Type:** Inter for the interface (body and display both), JetBrains Mono for
amounts, codes and the receipt. Both are TTFs, so both embed into the Rust binary
unchanged — the lane and the back office use the same two families.

**Rejected:** *Graphite & Lime* (acid lime on near-black — the most memorable, and my
own recommendation, but the user chose the calmer one) and *Bone & Forest* (warm
paper and deep green — the quietest, and the one that would age best).

## Left to the builder

- egui widget decomposition, and where lane state lives relative to `till-store`.
- The token and font port mechanics, and how drift from `globals.css` is caught.
- Table rendering for X and Z reports.
- Touch target sizing, subject to the latency budget above.
- Which generic ESC/POS profile is the default, and how simulated devices are
  driven in CI.
