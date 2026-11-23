import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import type { Device } from "@/data/types";
import { median, timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { icons } from "@/components/Icon";
import { Card, ConnectionBadge, DeviceGlyph, Figure, PageHeader, Panel, SectionLabel } from "@/components/ui";
import type { TetherModule } from "./types";

const SAMPLES = 40;

function Sparkline({ values }: { values: number[] }) {
  if (values.length < 6) return <span className="text-xs text-subtle">Sampling…</span>;
  const max = Math.max(...values);
  const min = Math.min(...values);
  const span = max - min || 1;
  const pts = values.map((v, i) => `${(i / (values.length - 1)) * 100},${92 - ((v - min) / span) * 84}`);
  const line = pts.join(" ");
  return (
    <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-8 w-full" aria-hidden>
      <polygon points={`0,100 ${line} 100,100`} fill="var(--accent-soft)" />
      <polyline points={line} fill="none" stroke="var(--accent)" strokeWidth="1.6" vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

/** Rolling window of each peer's latency, sampled as the daemon reports it. */
function useLatencyHistory(devices: Device[]) {
  const history = useRef(new Map<string, number[]>());
  const [, bump] = useState(0);
  useEffect(() => {
    for (const d of devices) {
      if (d.latencyMs === null || d.isMe || d.connection === "offline") continue;
      const arr = history.current.get(d.id) ?? [];
      arr.push(d.latencyMs);
      history.current.set(d.id, arr.slice(-SAMPLES));
    }
    bump((n) => n + 1);
  }, [devices]);
  return history.current;
}

/** How far each observer is behind each author. Offline devices fall behind. */
function lag(observer: Device, author: Device): number {
  if (observer.id === author.id || observer.connection !== "offline") return 0;
  return Math.max(0, Math.round(author.seq * 0.3));
}

function Tile({ label, value, decimals = 0, suffix, hint }: { label: string; value: number; decimals?: number; suffix?: string; hint: string }) {
  return (
    <Card pad={20}>
      <p className="text-[11px] font-medium tracking-[0.14em] text-subtle uppercase">{label}</p>
      <div className="mt-2 flex items-baseline gap-1.5">
        <Figure value={value} decimals={decimals} size={40} />
        {suffix && <span className="text-sm text-muted-foreground">{suffix}</span>}
      </div>
      <p className="mt-1 text-xs text-muted-foreground">{hint}</p>
    </Card>
  );
}

function DiagnosticsView() {
  const { devices, ops } = useTether();
  const history = useLatencyHistory(devices);
  const byId = useMemo(() => new Map(devices.map((d) => [d.id, d])), [devices]);
  const peers = devices.filter((d) => !d.isMe);
  const online = peers.filter((d) => d.connection !== "offline");
  const rtt = median(online.map((d) => d.latencyMs ?? 0));
  const behind = devices.reduce((n, o) => n + devices.reduce((m, a) => m + lag(o, a), 0), 0);

  return (
    <div>
      <PageHeader eyebrow="Diagnostics" title="Under the hood." subtitle="Live connections, replication state and the operation log that keeps every device in agreement." />

      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <Tile label="Peers online" value={online.length} suffix={`of ${peers.length}`} hint="Paired and reachable" />
        <Tile label="Median latency" value={rtt} decimals={1} suffix="ms" hint="Round trip to peers" />
        <Tile label="Replication lag" value={behind} suffix="ops" hint={behind ? "Offline devices will catch up" : "Everyone is current"} />
        <Tile label="Simulator" value={10000} suffix="seeds" hint="Last run: 0 invariant violations" />
      </div>

      <section className="mt-12">
        <SectionLabel>Peers</SectionLabel>
        <Panel className="divide-y divide-border">
          {peers.map((p) => (
            <div key={p.id} className="grid grid-cols-[minmax(0,220px)_minmax(0,160px)_minmax(0,1fr)_80px] items-center gap-5 px-5 py-4">
              <div className="flex min-w-0 items-center gap-3">
                <DeviceGlyph platform={p.platform} size={34} online={p.connection !== "offline"} />
                <span className="truncate text-sm font-medium">{p.name}</span>
              </div>
              <ConnectionBadge connection={p.connection} />
              <Sparkline values={history.get(p.id) ?? []} />
              <span className="text-right font-mono text-xs">{p.connection === "offline" || p.latencyMs === null ? "—" : `${p.latencyMs.toFixed(1)} ms`}</span>
            </div>
          ))}
        </Panel>
      </section>

      <section className="mt-12">
        <SectionLabel>Version vectors</SectionLabel>
        <Panel className="overflow-x-auto">
          <table className="w-full text-[13px]">
            <thead>
              <tr className="text-left text-[11px] tracking-[0.1em] text-subtle uppercase">
                <th className="px-5 py-3.5 font-medium">Holds from</th>
                {devices.map((a) => (
                  <th key={a.id} className="px-4 py-3.5 font-medium">
                    {a.name}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {devices.map((o) => (
                <tr key={o.id} className="border-t border-border">
                  <td className="px-5 py-3.5 font-medium">{o.name}</td>
                  {devices.map((a) => {
                    const l = lag(o, a);
                    return (
                      <td key={a.id} className={cn("px-4 py-3.5 font-mono text-xs", l ? "text-muted-foreground" : "text-foreground")}>
                        {a.seq - l}
                        {l > 0 && <span className="ml-2 text-destructive">−{l}</span>}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </Panel>
        <p className="mt-3 px-1 text-xs text-subtle">Each cell is the highest sequence number a device holds from an author. Peers swap these on connect and send only what is missing.</p>
      </section>

      <section className="mt-12">
        <SectionLabel>Operation log</SectionLabel>
        <Panel className="overflow-hidden">
          <AnimatePresence initial={false}>
            {ops.slice(0, 12).map((o) => (
              <motion.div
                key={`${o.device}:${o.seq}`}
                layout="position"
                initial={{ opacity: 0, y: -10, backgroundColor: "var(--muted)" }}
                animate={{ opacity: 1, y: 0, backgroundColor: "rgba(0,0,0,0)" }}
                transition={{ type: "spring", duration: 0.6, bounce: 0.1, backgroundColor: { duration: 1.6 } }}
                className="grid grid-cols-[minmax(0,170px)_70px_minmax(0,1fr)_110px_80px] items-center gap-4 border-b border-border px-5 py-3 font-mono text-xs last:border-b-0"
              >
                <span className="truncate font-sans text-[13px]">{byId.get(o.device)?.name}</span>
                <span className="text-muted-foreground">#{o.seq}</span>
                <span className="truncate text-muted-foreground">{o.kind}</span>
                <span className="text-subtle" title="Hybrid logical clock">
                  {o.hlc}
                </span>
                <span className="text-right font-sans text-muted-foreground">{timeAgo(o.ts)}</span>
              </motion.div>
            ))}
          </AnimatePresence>
        </Panel>
      </section>
    </div>
  );
}

export const diagnosticsModule: TetherModule = {
  id: "diagnostics",
  label: "Diagnostics",
  icon: icons.pulse,
  group: "Network",
  summary: "Live latency, version vectors and the replicated operation log.",
  status: "preview",
  view: DiagnosticsView,
};
