import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import {
  absMoney, clock, clockWithDay, groupMoney, methodLabel, plainQty, splitShifts, varianceKind,
  type CounterResponse, type CounterShift, type ShiftSale, type ShiftSalesResponse,
} from "@/lib/counter-view";

/**
 * Read-only presentation of the per-shift till roll-up. No inputs, no
 * buttons, no mutations: a fiscalised sale is a record. The variance leads
 * each closed shift; the shop total sits underneath.
 */

const MONO = "font-mono tabular-nums";
const KICKER = "font-mono text-[0.65rem] uppercase tracking-[0.12em] text-ink-mute";

function Tag({ tone, children }: { tone: "ok" | "bad" | "warn" | "over" | "live" | "mute"; children: React.ReactNode }) {
  const cls = {
    ok: "bg-success-soft text-success",
    bad: "bg-danger-soft text-danger",
    warn: "bg-warning-soft text-warning",
    over: "bg-info-soft text-info",
    mute: "bg-surface-3 text-ink-soft",
    // hi-vis is a ground for dark text only.
    live: "bg-hi-vis text-ink",
  }[tone];
  return <span className={`${MONO} rounded-full px-2.5 py-1 text-[0.65rem] font-semibold uppercase tracking-[0.09em] ${cls}`}>{children}</span>;
}

function Who({ s }: { s: CounterShift }) {
  return (
    <div className="font-semibold text-base text-ink">
      <span className={`${MONO} mr-2 rounded-full border border-line bg-surface-3 px-2 py-0.5 text-[0.65rem] uppercase tracking-[0.08em] text-ink-soft`}>
        {s.lane.name}
      </span>
      {s.cashier.name ?? "Unknown cashier"}
    </div>
  );
}

function Split({ s }: { s: CounterShift }) {
  const hasChange = !/^0+(\.0+)?$/.test(s.changeGiven);
  return (
    <div className="flex flex-col gap-1.5 text-sm text-ink-soft">
      {s.methods.map((m) => (
        <div key={m.method} className="flex justify-between">
          <span>{methodLabel(m.method)} taken</span>
          <b className={`${MONO} font-medium text-ink`}>{groupMoney(m.tendered)}</b>
        </div>
      ))}
      {hasChange && (
        <div className="flex justify-between">
          <span>Change given</span>
          <b className={`${MONO} font-medium text-ink`}>−{groupMoney(s.changeGiven)}</b>
        </div>
      )}
    </div>
  );
}

function Verdict({ s, now, tz }: { s: CounterShift; now: Date; tz?: string }) {
  const lane = s.lane.name;
  switch (s.state) {
    case "complete": {
      const v = s.variance;
      if (!v || v.diff === null || v.declared === null) {
        return (
          <Row tone="mute" tag="No count">
            <span className="font-semibold text-ink-soft">No cash count recorded</span>
            <Why> · the drawer was not counted at close, so there is nothing to compare</Why>
          </Row>
        );
      }
      const kind = varianceKind(v.diff);
      const why = `expected ${groupMoney(v.expected)} in the drawer, counted ${groupMoney(v.declared)}`;
      if (kind === "short")
        return (
          <Row tone="bad" tag="Needs a word">
            <span className={`${MONO} font-semibold text-danger`}>Short {absMoney(v.diff)}</span>
            <Why> · {why}</Why>
          </Row>
        );
      if (kind === "over")
        return (
          <Row tone="over" tag="Check the count">
            <span className={`${MONO} font-semibold text-info`}>Over {absMoney(v.diff)}</span>
            <Why> · {why}. A counting slip or an unrecorded sale, not a loss.</Why>
          </Row>
        );
      return (
        <Row tone="ok" tag="Settled">
          <span className={`${MONO} font-semibold text-success`}>Balanced</span>
          <Why> · {why}</Why>
        </Row>
      );
    }
    case "short_count": {
      const gap = (s.tillSaleCount ?? 0) - s.saleCount;
      return (
        <Row tone="warn" tag="Sales missing">
          <span className={`${MONO} font-semibold text-warning`}>Variance withheld</span>
          <Why>
            {" "}· {lane} reports {s.tillSaleCount} sales, we hold {s.saleCount} — {gap} not here yet
            (refused, or still queued on the till)
          </Why>
        </Row>
      );
    }
    case "unknown":
      return (
        <Row tone="mute" tag="Takings only">
          <span className={`${MONO} font-semibold text-ink-soft`}>Variance not available</span>
          <Why> · this shift closed before the till reported its own sale count, so completeness cannot be checked</Why>
        </Row>
      );
    case "mismatch":
      return (
        <Row tone="bad" tag="Data fault" loud>
          <span className={`${MONO} font-semibold text-danger`}>Variance withheld — data fault</span>
          <Why>
            {" "}· the sale count agrees with the till but the money does not add up. This is a fault in the records, not a
            cash shortage. Do not act on this shift until it is looked at.
          </Why>
        </Row>
      );
    case "stale":
      return <Stuck s={s} now={now} tz={tz} />;
    default:
      return null;
  }
}

function Why({ children }: { children: React.ReactNode }) {
  return <span className="text-[0.8125rem] text-ink-mute">{children}</span>;
}

function Row({ tone, tag, loud, children }: { tone: "ok" | "bad" | "warn" | "over" | "mute"; tag: string; loud?: boolean; children: React.ReactNode }) {
  return (
    <div className={`flex flex-wrap items-center justify-between gap-3.5 border-t border-line px-5 py-3 ${loud ? "bg-danger-soft" : ""}`}>
      <div>{children}</div>
      <Tag tone={tone}>{tag}</Tag>
    </div>
  );
}

const BORDER: Record<string, string> = {
  complete: "border-l-success",
  short_count: "border-l-warning",
  unknown: "border-l-line-strong",
  mismatch: "border-l-danger",
};

/** Presentational: the sales of one shift. Amounts are the stored strings, re-grouped as text only. */
export function ShiftSales({ data, tz }: { data: ShiftSalesResponse; tz?: string }) {
  return (
    <div className="border-t border-line bg-surface-2 px-5 py-3">
      <p className="mb-3 text-[0.8125rem] text-warning">{data.record.note}</p>
      {data.truncated && (
        <p role="status" className="mb-3 text-[0.8125rem] text-ink-soft">Showing first {data.sales.length} of {data.salesTotal} sales.</p>
      )}
      {data.sales.length === 0 && <p className="text-sm text-ink-soft">No sales held for this shift.</p>}
      {data.sales.map((x: ShiftSale) => (
        <div
          key={x.id}
          data-sale={x.invcNo}
          {...(x.voidedAt ? { "data-voided": "true" } : {})}
          className={`mb-3 rounded-[var(--radius)] border border-line bg-surface px-4 py-3 ${x.voidedAt ? "border-l-4 border-l-danger" : ""}`}
        >
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <div className={`${MONO} text-xs text-ink-mute`}>
              #{x.invcNo} · {clock(x.soldAt, tz)}
              {x.voidedAt && <span className="ml-2"><Tag tone="bad">Voided</Tag></span>}
            </div>
            {x.voidedAt ? (
              <div className="text-right">
                <div className={`${MONO} font-medium text-ink-mute line-through`}>{groupMoney(x.totAmt)}</div>
                <div className="text-[0.75rem] text-danger">Not counted in takings</div>
              </div>
            ) : (
              <div className={`${MONO} font-semibold text-ink`}>{groupMoney(x.totAmt)}</div>
            )}
          </div>
          <table className="mt-2 w-full text-sm">
            <thead>
              <tr className={KICKER}>
                <th className="text-left font-normal">Item</th>
                <th className="text-right font-normal">Qty</th>
                <th className="text-right font-normal">Price</th>
                <th className="text-right font-normal">Line</th>
                <th className="text-right font-normal">Taxable</th>
              </tr>
            </thead>
            <tbody>
              {x.lines.map((l) => (
                <tr key={l.itemSeq} className="text-ink-soft">
                  <td>{l.name}</td>
                  <td className={`${MONO} text-right`}>{plainQty(l.qty)}</td>
                  <td className={`${MONO} text-right`}>{groupMoney(l.prc)}</td>
                  <td className={`${MONO} text-right text-ink`}>{l.splyAmt === null ? "—" : groupMoney(l.splyAmt)}</td>
                  <td className={`${MONO} text-right`}>{l.taxblAmt === null ? "—" : groupMoney(l.taxblAmt)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="mt-2 flex flex-wrap justify-between gap-2 text-xs text-ink-mute">
            <span>{x.tenders.map((t) => `${methodLabel(t.method)} ${groupMoney(t.amount)}`).join(" · ")}</span>
            <span className={MONO}>Tax {groupMoney(x.totTaxAmt)}</span>
          </div>
        </div>
      ))}
    </div>
  );
}

function ShiftSalesLoader({ slug, shiftId, tz }: { slug: string; shiftId: string; tz?: string }) {
  const { data, isLoading, isError } = useQuery<ShiftSalesResponse>({
    queryKey: ["counter-shift", slug, shiftId],
    queryFn: async () => {
      const r = await fetch(`/api/${slug}/counter/${encodeURIComponent(shiftId)}`);
      if (!r.ok) throw new Error("Could not load the sales");
      return r.json();
    },
  });
  if (isLoading) return <p className="border-t border-line px-5 py-3 text-sm text-ink-soft">Loading sales…</p>;
  if (isError || !data) return <p role="alert" className="border-t border-line px-5 py-3 text-sm text-danger">Could not load this shift&rsquo;s sales.</p>;
  return <ShiftSales data={data} tz={tz} />;
}

// The only interaction on the page: show or hide. It changes nothing on the server.
function Expander({ slug, shiftId, tz }: { slug: string; shiftId: string; tz?: string }) {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="w-full border-t border-line px-5 py-2 text-left text-sm text-ink-soft hover:bg-surface-3"
      >
        {open ? "Hide sales" : "Show sales"}
      </button>
      {open && <ShiftSalesLoader slug={slug} shiftId={shiftId} tz={tz} />}
    </>
  );
}

function ClosedShift({ s, now, tz, slug }: { s: CounterShift; now: Date; tz?: string; slug?: string }) {
  let border = BORDER[s.state];
  if (s.state === "complete" && s.variance?.diff) {
    const k = varianceKind(s.variance.diff);
    border = k === "short" ? "border-l-danger" : k === "over" ? "border-l-info" : "border-l-success";
  }
  return (
    <article data-state={s.state} className={`my-2.5 overflow-hidden rounded-[var(--radius)] border border-line border-l-4 bg-surface ${border}`}>
      <div className="grid gap-4 px-5 py-4 md:grid-cols-[1.5fr_1.3fr_1fr] md:gap-[18px]">
        <div>
          <Who s={s} />
          <div className={`${MONO} mt-0.5 text-xs text-ink-mute`}>
            {clock(s.openedAt, tz)} → {s.closedAt ? clock(s.closedAt, tz) : "—"} · {s.saleCount} sales
          </div>
        </div>
        <Split s={s} />
        <div className="md:text-right">
          <div className={KICKER}>Took</div>
          <div className={`${MONO} mt-0.5 text-xl font-semibold text-ink`}>{groupMoney(s.takings)}</div>
        </div>
      </div>
      <Verdict s={s} now={now} tz={tz} />
      {slug && <Expander slug={slug} shiftId={s.id} tz={tz} />}
    </article>
  );
}

function Stuck({ s, now, tz }: { s: CounterShift; now: Date; tz?: string }) {
  const reached = s.lastReachedAt
    ? `lane not reachable since ${clockWithDay(s.lastReachedAt, now, tz)}`
    : "this lane has never been reachable";
  return (
    <div className="flex flex-wrap items-center justify-between gap-3.5 px-5 py-3.5">
      <div>
        <div className="font-semibold text-base text-ink">
          <span className={`${MONO} mr-2 rounded-full border border-line bg-surface-3 px-2 py-0.5 text-[0.65rem] uppercase tracking-[0.08em] text-ink-soft`}>{s.lane.name}</span>
          {s.cashier.name ?? "Unknown cashier"} · still open from {clockWithDay(s.openedAt, now, tz)}
        </div>
        <div className={`${MONO} mt-0.5 text-xs text-ink-mute`}>
          {s.saleCount} sales · {groupMoney(s.takings)} · {reached}. It may have closed on the till with the close never arriving.
        </div>
      </div>
      <Tag tone="bad">Stuck — check this lane</Tag>
    </div>
  );
}

function LiveShift({ s, now, tz }: { s: CounterShift; now: Date; tz?: string }) {
  if (s.state === "stale") {
    return (
      <article data-state="stale" className="my-2.5 rounded-[var(--radius)] border border-line border-l-4 border-l-danger bg-danger-soft">
        <Stuck s={s} now={now} tz={tz} />
      </article>
    );
  }
  return (
    <article data-state="open" className="my-2.5 flex flex-wrap items-center justify-between gap-3.5 rounded-[var(--radius)] border border-line bg-surface px-5 py-3.5">
      <div>
        <div className="font-semibold text-base text-ink">
          <span className={`${MONO} mr-2 rounded-full border border-line bg-surface-3 px-2 py-0.5 text-[0.65rem] uppercase tracking-[0.08em] text-ink-soft`}>{s.lane.name}</span>
          {s.cashier.name ?? "Unknown cashier"} · open since {clock(s.openedAt, tz)}
        </div>
        <div className={`${MONO} mt-0.5 text-xs text-ink-mute`}>
          {s.saleCount} sales · {groupMoney(s.takings)} so far{s.lastReachedAt ? ` · lane reachable ${clock(s.lastReachedAt, tz)}` : ""}
        </div>
      </div>
      <Tag tone="live">Trading</Tag>
    </article>
  );
}

export function CounterView({ data, now = new Date(), tz, dayLabel, slug }: { data: CounterResponse; now?: Date; tz?: string; dayLabel?: string; slug?: string }) {
  const { closed, live } = splitShifts(data.shifts);
  const stale = live.filter((s) => s.state === "stale");
  const label = dayLabel ?? data.day;
  return (
    <div>
      {stale.length > 0 && (
        <div role="alert" className="mb-4 rounded-[var(--radius)] border border-line border-l-4 border-l-danger bg-danger-soft px-5 py-3 text-sm text-danger">
          <b>{stale.length === 1 ? "1 shift looks stuck open" : `${stale.length} shifts look stuck open`}</b> — see “Trading now” below.
        </div>
      )}

      <h2 className={`${KICKER} mb-3 mt-2 font-semibold`}>Closed shifts · {label}</h2>
      {closed.length === 0 && (
        <p className="rounded-[var(--radius)] border border-line bg-surface px-5 py-4 text-sm text-ink-soft">No shift has closed on this day yet.</p>
      )}
      {closed.map((s) => <ClosedShift key={s.id} s={s} now={now} tz={tz} slug={slug} />)}

      <div className="mt-3.5 flex flex-wrap items-baseline justify-between gap-4 rounded-[var(--radius)] border border-line bg-surface-2 px-5 py-4">
        <div>
          <div className={KICKER}>{label}, all lanes</div>
          {!data.totalQualified && (
            <div className="mt-1 text-[0.8125rem] text-warning">Incomplete — some shifts are still open or not yet verified</div>
          )}
        </div>
        <div className={`${MONO} text-2xl font-semibold text-ink`}>{groupMoney(data.total)}</div>
      </div>

      {live.length > 0 && (
        <>
          <h2 className={`${KICKER} mb-3 mt-8 font-semibold`}>Trading now</h2>
          {live.map((s) => <LiveShift key={s.id} s={s} now={now} tz={tz} />)}
        </>
      )}
      {data.shifts.length === 0 && (
        <p className="mt-4 text-sm text-ink-soft">No shifts opened on this day.</p>
      )}
    </div>
  );
}
