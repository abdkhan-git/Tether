// What used to be the Diagnostics page, now part of Devices: round-trip
// history, version vectors and the operation log. Folded away by default.

import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import { opsOf } from "@/data/map";
import type { Device } from "@/data/types";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { Icon, icons } from "@/components/Icon";
import { Panel, SectionLabel } from "@/components/ui";

const SAMPLES = 40;

export function Sparkline({ values }: { values: number[] }) {
  if (values.length < 6) return <span className="text-xs text-subtle">Sampling…</span>;
  const max = Math.max(...values);
  const min = Math.min(...values);
  const span = max - min || 1;
  const pts = values.map((v, i) => `${(i / (values.length - 1)) * 100},${92 - ((v - min) / span) * 84}`);
  const line = pts.join(" ");
  return (
    <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-7 w-full" aria-hidden>
      <polyline points={line} fill="none" stroke="var(--muted-foreground)" strokeWidth="1.4" vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

/** Rolling window of each peer's round trip, sampled as the daemon reports it. */
export function useLatencyHistory(devices: Device[]) {
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

/** How far each observer is behind each author in the preview. Offline devices fall behind. */
function lag(observer: Device, author: Device): number {
  if (observer.id === author.id || observer.connection !== "offline") return 0;
  return Math.max(0, Math.round(author.seq * 0.3));
}

export function SyncDetails() {
  const { devices, ops: previewOps, clips, backend } = useTether();
  const [open, setOpen] = useState(false);
  const live = backend.live;
  const byId = useMemo(() => new Map(devices.map((d) => [d.id, d])), [devices]);
  const observers = live ? devices.filter((d) => d.isMe) : devices;
  const ops = useMemo(() => (live ? opsOf(clips) : previewOps), [live, clips, previewOps]);

  return (
    <section className="mt-12">
      <button onClick={() => setOpen(!open)} aria-expanded={open} className="mb-3 flex items-center gap-2 px-1 text-[13px] font-semibold text-foreground">
        <Icon icon={icons.chevronRight} size={14} className={cn("transition-transform duration-200", open && "rotate-90")} />
        Sync details
        <span className="font-normal text-subtle">Version vectors and the operation log that keep every device in agreement</span>
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }} exit={{ opacity: 0, height: 0 }} className="overflow-hidden">
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
                  {observers.map((o) => (
                    <tr key={o.id} className="border-t border-border">
                      <td className="px-5 py-3.5 font-medium">{o.name}</td>
                      {devices.map((a) => {
                        const l = live ? 0 : lag(o, a);
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
            <p className="mt-3 mb-8 px-1 text-xs text-subtle">
              Each cell is the highest sequence number a device holds from an author. Peers swap these on connect and send only what is missing.
              {live && " Only this device's row is shown: other devices' vectors are not reported to the app yet."}
            </p>

            <SectionLabel>Operation log</SectionLabel>
            <Panel className="overflow-hidden">
              {ops.slice(0, 12).map((o) => (
                <div key={`${o.device}:${o.seq}`} className="grid grid-cols-[minmax(0,170px)_70px_minmax(0,1fr)_110px_80px] items-center gap-4 border-b border-border px-5 py-3 font-mono text-xs last:border-b-0">
                  <span className="truncate font-sans text-[13px]">{byId.get(o.device)?.name}</span>
                  <span className="text-muted-foreground">#{o.seq}</span>
                  <span className="truncate text-muted-foreground">{o.kind}</span>
                  <span className="text-subtle" title="Hybrid logical clock">
                    {o.hlc}
                  </span>
                  <span className="text-right font-sans text-muted-foreground">{timeAgo(o.ts)}</span>
                </div>
              ))}
              {!ops.length && <p className="px-5 py-8 text-center text-[13px] text-subtle">Nothing in the log yet.</p>}
            </Panel>
          </motion.div>
        )}
      </AnimatePresence>
    </section>
  );
}
