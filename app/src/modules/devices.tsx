import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { errorText } from "@/data/daemon";
import { useTether } from "@/data/store";
import type { Device } from "@/data/types";
import { timeAgo } from "@/lib/format";
import { osLabel } from "@/lib/platform";
import SlideCommit from "@/components/rb/SlideCommit";
import { Modal } from "@/components/Modal";
import { Icon, icons } from "@/components/Icon";
import { Card, Chip, ConnectionBadge, DeviceGlyph, EmptyState, IconButton, PageHeader, Panel, PrimaryAction, SectionLabel, useColors } from "@/components/ui";
import type { TetherModule } from "./types";

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

function DeviceCard({ d, onRevoke }: { d: Device; onRevoke: (d: Device) => void }) {
  const offline = d.connection === "offline";
  const detail = d.isMe ? "—" : offline ? (d.lastSeen ? `seen ${timeAgo(d.lastSeen)}` : "offline") : d.latencyMs !== null ? `${d.latencyMs.toFixed(1)} ms` : "—";

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
            {d.platform ? osLabel[d.platform] : "Paired device"} · {d.seq.toLocaleString()} {d.seq === 1 ? "operation" : "operations"}
          </p>
        </div>

        <div className="mt-5 flex items-center justify-between border-t border-border pt-4">
          <ConnectionBadge connection={d.connection} />
          <span className="font-mono text-xs text-muted-foreground">{detail}</span>
        </div>
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
  const { devices, nearby, setPairOpen } = useTether();
  const [revoking, setRevoking] = useState<Device | null>(null);
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
            <DeviceCard key={d.id} d={d} onRevoke={setRevoking} />
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
      <RevokeSheet device={revoking} onClose={() => setRevoking(null)} />
    </div>
  );
}

export const devicesModule: TetherModule = {
  id: "devices",
  label: "Devices",
  icon: icons.wifi,
  group: "Network",
  summary: "Pair, rename and revoke the machines that make up your Tether.",
  status: "stable",
  daemon: true,
  core: true,
  view: DevicesView,
  commands: () => [{ id: "pair", label: "Pair a new device", keywords: "add code connect", run: () => useTether.getState().setPairOpen(true) }],
};
