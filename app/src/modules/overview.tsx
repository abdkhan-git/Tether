import { useMemo, useRef, type RefObject } from "react";
import { motion } from "motion/react";
import { useTether } from "@/data/store";
import type { Device } from "@/data/types";
import { bytes, median, rate, timeAgo } from "@/lib/format";
import { contentOf } from "@/lib/content/detect";
import { cn } from "@/lib/cn";
import { AnimatedBeam } from "@/components/magic/animated-beam";
import { icons } from "@/components/Icon";
import { Card, DeviceGlyph, Figure, IconButton, Panel, PrimaryAction, SectionLabel, StatusDot, useColors } from "@/components/ui";
import type { TetherModule } from "./types";

/** Where each peer sits around this device, as percentages of the stage. */
const SLOTS = [
  { x: 11, y: 50 },
  { x: 89, y: 24 },
  { x: 89, y: 76 },
  { x: 50, y: 10 },
];

function Node({ device, innerRef, hub }: { device: Device; innerRef: RefObject<HTMLDivElement | null>; hub?: boolean }) {
  const offline = device.connection === "offline";
  return (
    <div className="flex flex-col items-center gap-2.5">
      <div ref={innerRef} className="relative">
        {hub && <span className="absolute -inset-3 animate-ping rounded-full border border-foreground/10" style={{ animationDuration: "3.2s" }} />}
        <div className={cn("rounded-full p-1", hub ? "bg-muted ring-1 ring-border-hover" : "")}>
          <DeviceGlyph platform={device.platform} size={hub ? 72 : 56} online={!offline} />
        </div>
        {!offline && !hub && <StatusDot connection={device.connection} className="absolute top-1 right-1" />}
      </div>
      <div className="text-center leading-tight whitespace-nowrap">
        <div className={cn("text-[13px] font-medium", offline && "text-subtle")}>{hub ? "This device" : device.name}</div>
        <div className="mt-0.5 font-mono text-[11px] text-subtle">
          {hub ? device.name : offline ? "offline" : `${device.latencyMs?.toFixed(1)} ms`}
        </div>
      </div>
    </div>
  );
}

function Constellation() {
  const { devices, me } = useTether();
  const { c } = useColors();
  const stage = useRef<HTMLDivElement>(null);
  const hubRef = useRef<HTMLDivElement>(null);
  const r0 = useRef<HTMLDivElement>(null);
  const r1 = useRef<HTMLDivElement>(null);
  const r2 = useRef<HTMLDivElement>(null);
  const r3 = useRef<HTMLDivElement>(null);
  const refs = [r0, r1, r2, r3];

  const hub = devices.find((d) => d.id === me)!;
  const peers = devices.filter((d) => d.id !== me).slice(0, SLOTS.length);

  return (
    <Card pad={0} className="h-full min-h-[340px]">
      <div className="absolute top-5 left-6 z-10">
        <p className="text-[11px] font-medium tracking-[0.14em] text-subtle uppercase">Your tether</p>
      </div>
      <div ref={stage} className="relative h-full min-h-[340px] w-full">
        {peers.map((p, i) => {
          const s = SLOTS[i];
          return (
            <div key={p.id} className="absolute -translate-x-1/2 -translate-y-1/2" style={{ left: `${s.x}%`, top: `${s.y}%` }}>
              <Node device={p} innerRef={refs[i]} />
            </div>
          );
        })}
        <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2">
          <Node device={hub} innerRef={hubRef} hub />
        </div>

        {peers.map((p, i) =>
          p.connection === "offline" ? (
            <AnimatedBeam key={p.id} containerRef={stage} fromRef={refs[i]} toRef={hubRef} curvature={i % 2 ? -34 : 34} pathColor={c.muted} pathOpacity={0.14} pathWidth={1.5} gradientStartColor="transparent" gradientStopColor="transparent" duration={60} />
          ) : (
            <AnimatedBeam
              key={p.id}
              containerRef={stage}
              fromRef={refs[i]}
              toRef={hubRef}
              reverse={i % 2 === 1}
              curvature={i % 2 ? -34 : 34}
              pathColor={c.muted}
              pathOpacity={0.22}
              pathWidth={1.5}
              gradientStartColor={c.ink}
              gradientStopColor={c.positive}
              duration={3.6 + i * 0.7}
              delay={i * 0.5}
            />
          ),
        )}
      </div>
    </Card>
  );
}

function Stat({ label, value, decimals = 0, suffix, hint }: { label: string; value: number; decimals?: number; suffix?: string; hint: string }) {
  return (
    <Card pad={20} className="flex-1">
      <p className="text-[11px] font-medium tracking-[0.14em] text-subtle uppercase">{label}</p>
      <div className="mt-2 flex items-baseline gap-1.5">
        <Figure value={value} decimals={decimals} size={46} />
        {suffix && <span className="text-sm text-muted-foreground">{suffix}</span>}
      </div>
      <p className="mt-1.5 text-xs text-muted-foreground">{hint}</p>
    </Card>
  );
}

function OverviewView() {
  const { devices, clips, transfers, me, setPairOpen, setActiveModule, toast } = useTether();
  const names = useMemo(() => new Map(devices.map((d) => [d.id, d.name])), [devices]);
  const peers = devices.filter((d) => !d.isMe);
  const online = peers.filter((d) => d.connection !== "offline");
  const rtt = median(online.map((d) => d.latencyMs ?? 0));
  const totalOps = devices.reduce((n, d) => n + d.seq, 0);
  const latest = [...clips].filter((c) => !c.sensitive).sort((a, b) => b.ts - a.ts).slice(0, 4);
  const active = transfers.find((t) => t.state === "active") ?? transfers.find((t) => t.state === "paused");

  const copy = (text: string) =>
    navigator.clipboard.writeText(text).then(
      () => toast("Copied to this device"),
      () => toast("Clipboard access was blocked"),
    );

  return (
    <div className="flex flex-col gap-10">
      <header className="flex flex-wrap items-end justify-between gap-6 pt-2">
        <div>
          <p className="mb-2 text-[13px] font-medium text-muted-foreground">Overview</p>
          <motion.h1 initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", duration: 0.7, bounce: 0.1 }} className="display text-[clamp(2.4rem,4.6vw,3.4rem)] text-foreground">
            Your devices, as one.
          </motion.h1>
          <p className="mt-4 max-w-lg text-base text-muted-foreground">
            {online.length} of {peers.length} paired devices are online. Whatever you copy on one is already waiting on the rest.
          </p>
        </div>
        <PrimaryAction icon={icons.add} onClick={() => setPairOpen(true)}>
          Pair a device
        </PrimaryAction>
      </header>

      <section className="grid gap-4 lg:grid-cols-[minmax(0,1.9fr)_minmax(0,1fr)]">
        <Constellation />
        <div className="flex flex-col gap-4">
          <Stat label="Round trip" value={rtt} decimals={1} suffix="ms" hint="Median to connected peers" />
          <Stat label="Operations synced" value={totalOps} hint="Across every device's log" />
          <Stat label="Replication lag" value={0} suffix="ops" hint="Everyone is up to date" />
        </div>
      </section>

      <section className="grid gap-8 lg:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)]">
        <div>
          <SectionLabel action={<button onClick={() => setActiveModule("clipboard")} className="text-xs text-muted-foreground transition-colors hover:text-foreground">View all</button>}>Latest from your devices</SectionLabel>
          <Panel className="divide-y divide-border overflow-hidden">
            {latest.map((c, i) => {
              const dev = devices.find((d) => d.id === c.device);
              return (
                <motion.div key={c.id} layout="position" initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", duration: 0.5, bounce: 0.15, delay: i * 0.04 }} className="group flex items-center gap-3.5 px-4 py-3.5 transition-colors hover:bg-muted/60">
                  {dev && <DeviceGlyph platform={dev.platform} size={34} online={dev.connection !== "offline"} />}
                  <div className="min-w-0 flex-1">
                    <p className={cn("truncate text-[13.5px]", ["code", "json"].includes(contentOf(c.text).kind) && "font-mono text-xs")}>{c.text.split("\n")[0]}</p>
                    <p className="mt-0.5 text-xs text-subtle">
                      {c.device === me ? "This device" : names.get(c.device)} · {timeAgo(c.ts)}
                    </p>
                  </div>
                  <div className="opacity-0 transition-opacity duration-200 group-hover:opacity-100 focus-within:opacity-100">
                    <IconButton icon={icons.copy} label="Copy to this device" onClick={() => void copy(c.text)} />
                  </div>
                </motion.div>
              );
            })}
          </Panel>
        </div>

        <div>
          <SectionLabel action={<button onClick={() => setActiveModule("transfers")} className="text-xs text-muted-foreground transition-colors hover:text-foreground">Open</button>}>Transfer</SectionLabel>
          {active ? (
            <Card pad={20}>
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <p className="truncate text-[13.5px] font-medium">{active.name}</p>
                  <p className="mt-0.5 text-xs text-subtle">
                    {names.get(active.from) === names.get(me) ? "This device" : names.get(active.from)} → {active.to === me ? "This device" : names.get(active.to)}
                  </p>
                </div>
                <span className="font-mono text-xs text-muted-foreground">{active.state === "active" ? rate(active.speed) : "Paused"}</span>
              </div>
              <div className="mt-5 h-1.5 overflow-hidden rounded-full bg-muted">
                <motion.div className="h-full rounded-full bg-accent" animate={{ width: `${(active.bytesDone / active.size) * 100}%` }} transition={{ type: "spring", duration: 0.8, bounce: 0 }} />
              </div>
              <p className="mt-2.5 text-xs text-muted-foreground">
                {bytes(active.bytesDone)} of {bytes(active.size)}
              </p>
            </Card>
          ) : (
            <Panel className="flex h-[132px] items-center justify-center text-[13px] text-subtle">No transfers right now</Panel>
          )}
        </div>
      </section>
    </div>
  );
}

export const overviewModule: TetherModule = {
  id: "overview",
  label: "Overview",
  icon: icons.home,
  group: "Sync",
  summary: "Your devices, their connections and what just synced.",
  status: "stable",
  core: true,
  view: OverviewView,
};
