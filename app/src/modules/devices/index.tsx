import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { errorText } from "@/data/daemon";
import { useTether } from "@/data/store";
import type { Device, Health } from "@/data/types";
import { bytes, timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { osLabel } from "@/lib/platform";
import SlideCommit from "@/components/rb/SlideCommit";
import { Modal } from "@/components/Modal";
import { Icon, icons } from "@/components/Icon";
import { Button, Card, Chip, ConnectionBadge, DeviceGlyph, EmptyState, IconButton, PageHeader, Panel, PrimaryAction, SectionLabel, useColors } from "@/components/ui";
import type { TetherModule } from "../types";
import { Sparkline, SyncDetails, useLatencyHistory } from "./sync-details";

/** How often the page says it is still looking (the daemon keeps fast sampling for 10 s). */
const WATCH_EVERY_MS = 4000;

function uptime(s: number): string {
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))} min`;
  if (s < 86400) return `${Math.round(s / 3600)} h`;
  return `${Math.floor(s / 86400)} d ${Math.round((s % 86400) / 3600)} h`;
}

/** One measure as a thin bar. Neutral unless it is a problem: then red. */
function Meter({ label, value, detail, alarm }: { label: string; value: number; detail: string; alarm?: boolean }) {
  const pct = Math.max(0, Math.min(100, value));
  return (
    <div>
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span className="text-muted-foreground">{label}</span>
        <span className={cn("font-mono tabular-nums", alarm ? "text-destructive" : "text-foreground")}>{detail}</span>
      </div>
      <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-muted" role="meter" aria-label={label} aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
        <motion.div className={cn("h-full rounded-full", alarm ? "bg-destructive" : "bg-foreground/55")} animate={{ width: `${pct}%` }} transition={{ type: "spring", duration: 0.8, bounce: 0 }} />
      </div>
    </div>
  );
}

/** Low storage, as the daemon judges it: under 5 % or under 5 GB free. */
function lowStorage(h: Health): boolean {
  return h.diskTotal > 0 && (h.diskFree < 5e9 || h.diskFree * 20 < h.diskTotal);
}

function HealthPanel({ h }: { h: Health }) {
  return (
    <div className="mt-5 flex flex-col gap-3">
      <Meter label="CPU" value={h.cpu} detail={`${Math.round(h.cpu)}%`} alarm={h.cpu >= 95} />
      <Meter label="Memory" value={(h.memUsed / Math.max(1, h.memTotal)) * 100} detail={`${bytes(h.memUsed)} of ${bytes(h.memTotal)}`} />
      <Meter label="Storage" value={(1 - h.diskFree / Math.max(1, h.diskTotal)) * 100} detail={`${bytes(h.diskFree)} free`} alarm={lowStorage(h)} />
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>Up {uptime(h.uptimeS)}</span>
        {h.battery && (
          <span className={cn("font-mono", h.battery.percent <= 10 && !h.battery.charging && "text-destructive")}>
            Battery {h.battery.percent}%{h.battery.charging ? ", charging" : ""}
          </span>
        )}
      </div>
    </div>
  );
}

const spring = { type: "spring", duration: 0.5, bounce: 0.15 } as const;

/** A device's name, renamed in place: click, type, Enter (Esc cancels). Synced to every device. */
function DeviceName({ d }: { d: Device }) {
  const { backend, toast } = useTether();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(d.name);

  const commit = () => {
    const next = name.trim();
    setEditing(false);
    if (!next || next === d.name) {
      setName(d.name);
      return;
    }
    backend.renameDevice(d.id, next).then(
      () => toast(`Renamed to ${next}`, "The new name shows on every device."),
      (e) => {
        setName(d.name);
        toast("Could not rename the device", errorText(e));
      },
    );
  };

  if (editing) {
    return (
      <input
        autoFocus
        value={name}
        maxLength={64}
        aria-label={`New name for ${d.name}`}
        onChange={(e) => setName(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") {
            setName(d.name);
            setEditing(false);
          }
        }}
        className="selectable w-full rounded-lg bg-muted px-2 py-0.5 text-[19px] font-medium tracking-tight outline-none"
      />
    );
  }
  return (
    <button
      onClick={() => {
        setName(d.name);
        setEditing(true);
      }}
      className="group flex max-w-full items-center gap-2 text-left text-[19px] font-medium tracking-tight"
      aria-label={`Rename ${d.name}`}
      title={d.name}
    >
      <span className="truncate">{d.name}</span>
      <Icon icon={icons.edit} size={14} className="shrink-0 text-subtle opacity-0 transition-opacity group-hover:opacity-100" />
    </button>
  );
}

function DeviceCard({ d, onRevoke, rtts }: { d: Device; onRevoke: (d: Device) => void; rtts: number[] }) {
  const { backend, toast } = useTether();
  const [waking, setWaking] = useState(false);
  const offline = d.connection === "offline";
  const quiet = !offline && (d.liveness === "suspect" || d.liveness === "down");
  const detail = d.isMe ? "—" : offline ? (d.lastSeen ? `seen ${timeAgo(d.lastSeen)}` : "offline") : d.latencyMs !== null ? `${d.latencyMs.toFixed(1)} ms` : "—";
  const wake = () => {
    setWaking(true);
    backend.wakeDevice(d.id).then(
      () => toast(`Woke ${d.name}`, "If it was asleep it should connect within a minute. If not, Wake on LAN may be off in its network adapter settings, or it is not on this network."),
      (e) => toast(`Could not wake ${d.name}`, errorText(e)),
    ).finally(() => setTimeout(() => setWaking(false), 5000));
  };

  return (
    <motion.li layout="position" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, scale: 0.96 }} transition={spring}>
      <Card pad={24} className="h-full">
        <div className="flex items-start justify-between gap-3">
          <DeviceGlyph platform={d.platform} size={52} online={!offline} />
          {d.isMe ? (
            <Chip>This device</Chip>
          ) : (
            <IconButton icon={icons.revoke} label="Revoke access" tone="danger" onClick={() => onRevoke(d)} />
          )}
        </div>

        <div className="mt-6">
          <DeviceName key={d.name} d={d} />
          <p className="mt-1 text-[13px] text-muted-foreground">
            {d.health?.os || (d.platform ? osLabel[d.platform] : "Paired device")} · {d.seq.toLocaleString()} {d.seq === 1 ? "operation" : "operations"}
          </p>
        </div>

        {quiet && (
          <p className="mt-4 rounded-xl bg-muted px-3 py-2 text-xs text-muted-foreground">
            {d.liveness === "down" ? "Not responding: it is probably asleep, switched off or out of range." : "Quieter than usual: it may be busy, or about to drop off."}
          </p>
        )}
        {d.health && !offline ? (
          <HealthPanel h={d.health} />
        ) : offline ? (
          <div className="mt-5 flex items-center justify-between gap-3 rounded-xl border border-dashed border-border px-3.5 py-3">
            <span className="text-xs text-muted-foreground">{d.wakeable ? "Asleep or switched off?" : "Its health shows once it is back online."}</span>
            {d.wakeable && (
              <Button size="sm" variant="outline" icon={icons.flash} onClick={wake} disabled={waking}>
                {waking ? "Sent" : "Wake"}
              </Button>
            )}
          </div>
        ) : null}

        <div className="mt-5 flex items-center justify-between border-t border-border pt-4">
          <ConnectionBadge connection={d.connection} />
          <span className="font-mono text-xs text-muted-foreground">{detail}</span>
        </div>
        {!d.isMe && !offline && (
          <div className="mt-2" title="Round trip, last two minutes">
            <Sparkline values={rtts} />
          </div>
        )}
        <p className="mt-3 font-mono text-[11px] tracking-wide text-subtle" title="Compare on both devices to confirm you paired with the right one">
          {d.fingerprint}
        </p>
      </Card>
    </motion.li>
  );
}

function RevokeSheet({ device, onClose }: { device: Device | null; onClose: () => void }) {
  const { revokeDevice, toast } = useTether();
  const { c } = useColors();
  return (
    <Modal open={!!device} onClose={onClose} title="Revoke access" width={440}>
      {device && (
        <div className="flex flex-col gap-6">
          <p className="-mt-3 text-[13.5px] text-muted-foreground">
            <span className="text-foreground">{device.name}</span> will be rejected from now on. What it already synced stays, because the log is immutable. You can pair it again later.
          </p>
          <div className="flex justify-center">
            <SlideCommit
              label="Slide to revoke"
              doneLabel="Revoked"
              width={340}
              height={56}
              trackColor={c.track}
              handleColor={c.danger}
              successColor={c.danger}
              dangerColor={c.danger}
              onConfirm={() => revokeDevice(device.id)}
              onDone={() => {
                toast(`${device.name} was revoked`);
                onClose();
              }}
              onError={(e: unknown) => {
                toast(`Could not revoke ${device.name}`, errorText(e));
                onClose();
              }}
            />
          </div>
        </div>
      )}
    </Modal>
  );
}

function DevicesView() {
  const { devices, nearby, setPairOpen, backend } = useTether();
  const [revoking, setRevoking] = useState<Device | null>(null);
  const history = useLatencyHistory(devices);

  // While this page is open, every device sends its health every 2 s.
  useEffect(() => {
    const say = () => document.visibilityState === "visible" && void backend.watchDevices().catch(() => {});
    say();
    const t = setInterval(say, WATCH_EVERY_MS);
    return () => clearInterval(t);
  }, [backend]);

  const others = devices.filter((d) => !d.isMe);
  const me = devices.filter((d) => d.isMe);
  const online = others.filter((d) => d.connection !== "offline").length;

  return (
    <div>
      <PageHeader
        eyebrow="Devices"
        title="Your machines."
        subtitle={
          others.length
            ? `${online} of ${others.length} paired ${others.length === 1 ? "device" : "devices"} online. Each one is identified by a key that never leaves it.`
            : "No other devices yet. Pair one and everything you copy follows you to it."
        }
        actions={
          <PrimaryAction icon={icons.add} onClick={() => setPairOpen(true)}>
            Pair a device
          </PrimaryAction>
        }
      />
      <SectionLabel>All devices</SectionLabel>
      <ul className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        <AnimatePresence initial={false}>
          {[...me, ...others].map((d) => (
            <DeviceCard key={d.id} d={d} onRevoke={setRevoking} rtts={history.get(d.id) ?? []} />
          ))}
        </AnimatePresence>
      </ul>
      {devices.length === 0 && (
        <Panel>
          <EmptyState title="Waiting for the Tether service" hint="Your devices appear here once it answers." />
        </Panel>
      )}
      {nearby.length > 0 && (
        <section className="mt-10">
          <SectionLabel>Nearby, not paired</SectionLabel>
          <Panel className="divide-y divide-border">
            {nearby.map((n) => (
              <div key={n.id} className="flex items-center gap-3 px-5 py-3.5">
                <DeviceGlyph platform={null} size={34} />
                <span className="min-w-0 flex-1 truncate text-sm">{n.name}</span>
                <span className="font-mono text-[11px] text-subtle">{n.id.slice(0, 12)}</span>
              </div>
            ))}
          </Panel>
          <p className="mt-3 px-1 text-xs text-subtle">These devices run Tether on your network. They can see nothing of yours until one of them enters a pairing code from this device.</p>
        </section>
      )}
      <SyncDetails />
      <RevokeSheet device={revoking} onClose={() => setRevoking(null)} />
    </div>
  );
}

export const devicesModule: TetherModule = {
  id: "devices",
  label: "Devices",
  icon: icons.wifi,
  group: "Network",
  summary: "Pair, rename and revoke your machines; see how each is doing and wake a sleeping one.",
  status: "stable",
  daemon: true,
  core: true,
  view: DevicesView,
  commands: () => [{ id: "pair", label: "Pair a new device", keywords: "add code connect", run: () => useTether.getState().setPairOpen(true) }],
};
