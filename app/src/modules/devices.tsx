import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import type { Device } from "@/data/types";
import { timeAgo } from "@/lib/format";
import { osLabel } from "@/lib/platform";
import SlideCommit from "@/components/rb/SlideCommit";
import { Modal } from "@/components/Modal";
import { Icon, icons } from "@/components/Icon";
import { Card, Chip, ConnectionBadge, DeviceGlyph, IconButton, PageHeader, PrimaryAction, SectionLabel, useColors } from "@/components/ui";
import type { TetherModule } from "./types";

const spring = { type: "spring", duration: 0.5, bounce: 0.15 } as const;

function DeviceCard({ d, onRevoke }: { d: Device; onRevoke: (d: Device) => void }) {
  const renameDevice = useTether((s) => s.renameDevice);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(d.name);
  const offline = d.connection === "offline";

  const commit = () => {
    const n = name.trim();
    if (n && n !== d.name) renameDevice(d.id, n);
    else setName(d.name);
    setEditing(false);
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
          {editing ? (
            <input
              autoFocus
              value={name}
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
          ) : (
            <button onClick={() => setEditing(true)} className="group flex items-center gap-2 text-left text-[19px] font-medium tracking-tight" aria-label={`Rename ${d.name}`}>
              {d.name}
              <Icon icon={icons.edit} size={14} className="text-subtle opacity-0 transition-opacity group-hover:opacity-100" />
            </button>
          )}
          <p className="mt-1 text-[13px] text-muted-foreground">{osLabel[d.platform]}</p>
        </div>

        <div className="mt-5 flex items-center justify-between border-t border-border pt-4">
          <ConnectionBadge connection={d.connection} />
          <span className="font-mono text-xs text-muted-foreground">{offline ? `seen ${timeAgo(d.lastSeen)}` : d.isMe ? "—" : `${d.latencyMs?.toFixed(1)} ms`}</span>
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
              onConfirm={() => new Promise((r) => setTimeout(r, 650))}
              onDone={() => {
                revokeDevice(device.id);
                toast(`${device.name} was revoked`);
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
  const { devices, setPairOpen } = useTether();
  const [revoking, setRevoking] = useState<Device | null>(null);
  const others = devices.filter((d) => !d.isMe);
  const me = devices.filter((d) => d.isMe);

  return (
    <div>
      <PageHeader
        eyebrow="Devices"
        title="Your machines."
        subtitle={`${others.filter((d) => d.connection !== "offline").length} of ${others.length} paired devices online. Each one is identified by a key that never leaves it.`}
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
  core: true,
  view: DevicesView,
  commands: () => [{ id: "pair", label: "Pair a new device", keywords: "add code connect", run: () => useTether.getState().setPairOpen(true) }],
};
