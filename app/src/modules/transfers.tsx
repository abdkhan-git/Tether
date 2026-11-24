import { useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import type { Device, Transfer } from "@/data/types";
import { bytes, eta, rate, timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { fileManager, os } from "@/lib/platform";
import { BorderBeam } from "@/components/magic/border-beam";
import { Icon, icons } from "@/components/Icon";
import { Button, Card, DeviceGlyph, EmptyState, IconButton, PageHeader, PrimaryAction, SectionLabel, useColors } from "@/components/ui";
import type { TetherModule } from "./types";

const CELLS = 72;
const spring = { type: "spring", duration: 0.5, bounce: 0.15 } as const;

/**
 * One cell per slice of the file. Cells filled before this attempt started are
 * "kept" (resume skipped them); cells filled since are "new".
 */
function ChunkMap({ t }: { t: Transfer }) {
  const done = Math.floor((t.bytesDone / t.size) * CELLS);
  const kept = Math.floor((t.resumedFrom / t.size) * CELLS);
  const inflight = t.state === "active" ? Math.min(6, CELLS - done) : 0;
  return (
    <div className="grid gap-[3px]" style={{ gridTemplateColumns: `repeat(${CELLS}, minmax(0, 1fr))` }} role="img" aria-label={`${Math.round((t.bytesDone / t.size) * 100)}% received`}>
      {Array.from({ length: CELLS }, (_, i) => {
        const state = i < kept ? "kept" : i < done ? "new" : i < done + inflight ? "flight" : "todo";
        return (
          <span
            key={i}
            className={cn(
              "h-3.5 rounded-[3px] transition-colors duration-300",
              state === "kept" && "bg-foreground/30",
              state === "new" && "bg-accent",
              state === "flight" && "animate-pulse bg-accent/50",
              state === "todo" && "bg-muted",
            )}
          />
        );
      })}
    </div>
  );
}

function TransferRow({ t, devices, me }: { t: Transfer; devices: Map<string, Device>; me: string }) {
  const { toggleTransfer, cancelTransfer, toast } = useTether();
  const { c } = useColors();
  const from = t.from === me ? "This device" : (devices.get(t.from)?.name ?? "Unknown");
  const to = t.to === me ? "This device" : (devices.get(t.to)?.name ?? "Unknown");
  const pct = Math.round((t.bytesDone / t.size) * 100);
  const live = t.state === "active";

  return (
    <motion.li layout="position" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, scale: 0.98 }} transition={spring} className="relative overflow-hidden rounded-[22px] border border-border bg-surface p-5">
      {live && <BorderBeam size={120} duration={7} colorFrom={c.positive} colorTo={c.ink} />}
      <div className="flex items-start gap-4">
        <span className="flex size-11 shrink-0 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
          <Icon icon={icons.file} size={20} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2.5">
            <span className="truncate text-[15px] font-medium">{t.name}</span>
            <span className="shrink-0 text-xs text-subtle">{bytes(t.size)}</span>
          </div>
          <p className="mt-0.5 flex items-center gap-1.5 text-[13px] text-muted-foreground">
            {from}
            <Icon icon={icons.chevronRight} size={13} />
            {to}
          </p>
        </div>
        <div className="flex items-center gap-0.5">
          {t.state !== "done" ? (
            <>
              <IconButton icon={live ? icons.pause : icons.play} label={live ? "Pause" : "Resume"} onClick={() => toggleTransfer(t.id)} />
              <IconButton icon={icons.close} label="Cancel" onClick={() => cancelTransfer(t.id)} />
            </>
          ) : (
            <Button size="sm" variant="outline" icon={icons.folder} onClick={() => toast(`Opened in ${fileManager[os]}`)}>
              Show in {fileManager[os]}
            </Button>
          )}
        </div>
      </div>

      <div className="mt-5 flex flex-col gap-2.5">
        <ChunkMap t={t} />
        <div className="flex items-center justify-between text-xs text-muted-foreground">
          {t.state === "done" ? (
            <span className="inline-flex items-center gap-1.5 text-foreground">
              <Icon icon={icons.tick} size={14} className="text-positive" /> Verified · {timeAgo(t.updated)}
            </span>
          ) : (
            <span>
              {pct}% · {bytes(t.bytesDone)} of {bytes(t.size)}
            </span>
          )}
          <span className="font-mono">
            {live && `${rate(t.speed)} · ${eta(t.size - t.bytesDone, t.speed)} left`}
            {t.state === "paused" && "Paused"}
          </span>
        </div>
        {t.resumedFrom > 0 && t.state !== "done" && <p className="text-xs text-subtle">Resumed. {bytes(t.resumedFrom)} was already on the receiver, so none of it was sent again.</p>}
      </div>
    </motion.li>
  );
}

function TransfersView() {
  const { transfers, devices, me, sendFile, toast } = useTether();
  const [over, setOver] = useState<string | null>(null);
  const picker = useRef<HTMLInputElement>(null);
  const target = useRef<string | null>(null);

  const byId = useMemo(() => new Map(devices.map((d) => [d.id, d])), [devices]);
  const peers = devices.filter((d) => !d.isMe);
  const active = transfers.filter((t) => t.state !== "done");
  const finished = transfers.filter((t) => t.state === "done");

  const send = (files: FileList | File[] | null, to: string) => {
    if (!files || !files.length) return;
    const list = Array.from(files);
    for (const f of list) sendFile(f.name, f.size, to);
    toast(`Sending ${list.length === 1 ? list[0].name : `${list.length} files`}`, `to ${byId.get(to)?.name}`);
  };

  const pick = (id: string | null) => {
    target.current = id;
    if (id) picker.current?.click();
    else toast("No device is online");
  };

  return (
    <div>
      <PageHeader
        eyebrow="Transfers"
        title="Send anything, anywhere."
        subtitle="Files travel straight between your devices in verified chunks, and pick up where they stopped."
        actions={
          <PrimaryAction icon={icons.upload} onClick={() => pick(peers.find((p) => p.connection !== "offline")?.id ?? null)}>
            Send a file
          </PrimaryAction>
        }
      />
      <input
        ref={picker}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          if (target.current) send(e.target.files, target.current);
          e.target.value = "";
        }}
      />

      <section className="mb-10">
        <SectionLabel>Drop a file on a device</SectionLabel>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-3">
          {peers.map((p) => {
            const offline = p.connection === "offline";
            const hot = over === p.id && !offline;
            return (
              <button
                key={p.id}
                disabled={offline}
                onClick={() => pick(p.id)}
                onDragOver={(e) => {
                  e.preventDefault();
                  setOver(p.id);
                }}
                onDragLeave={() => setOver(null)}
                onDrop={(e) => {
                  e.preventDefault();
                  setOver(null);
                  send(e.dataTransfer.files, p.id);
                }}
                className={cn("group rounded-[22px] border border-dashed p-4 text-left transition-[border-color,background-color,transform,opacity] duration-300 disabled:opacity-40", hot ? "scale-[1.02] border-foreground/40 bg-muted" : "border-border hover:border-border-hover hover:bg-muted/50")}
              >
                <span className="flex items-center gap-3.5">
                  <DeviceGlyph platform={p.platform} size={44} online={!offline} />
                  <span className="min-w-0">
                    <span className="block truncate text-sm font-medium">{p.name}</span>
                    <span className="block text-xs text-muted-foreground">{offline ? "Offline" : hot ? "Release to send" : "Drop or click"}</span>
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      </section>

      {transfers.length === 0 ? (
        <Card className="h-64">
          <EmptyState title="No transfers yet" hint="Drop a file on a device above to send it." />
        </Card>
      ) : (
        <div className="flex flex-col gap-10">
          {active.length > 0 && (
            <section>
              <SectionLabel>In progress</SectionLabel>
              <ul className="flex flex-col gap-3">
                <AnimatePresence initial={false}>
                  {active.map((t) => (
                    <TransferRow key={t.id} t={t} devices={byId} me={me} />
                  ))}
                </AnimatePresence>
              </ul>
              <div className="flex gap-5 px-1 pt-3 text-xs text-muted-foreground">
                {(
                  [
                    ["bg-foreground/30", "Already there"],
                    ["bg-accent", "Received"],
                    ["bg-muted", "Pending"],
                  ] as const
                ).map(([cls, label]) => (
                  <span key={label} className="inline-flex items-center gap-1.5">
                    <span className={cn("size-2 rounded-[2px]", cls)} />
                    {label}
                  </span>
                ))}
              </div>
            </section>
          )}
          {finished.length > 0 && (
            <section>
              <SectionLabel>Completed</SectionLabel>
              <ul className="flex flex-col gap-3">
                <AnimatePresence initial={false}>
                  {finished.map((t) => (
                    <TransferRow key={t.id} t={t} devices={byId} me={me} />
                  ))}
                </AnimatePresence>
              </ul>
            </section>
          )}
        </div>
      )}
    </div>
  );
}

export const transfersModule: TetherModule = {
  id: "transfers",
  label: "Transfers",
  icon: icons.upload,
  group: "Sync",
  summary: "Direct, resumable file transfers. Drag a file onto a device.",
  status: "preview",
  note: "Becomes Drop · P3",
  view: TransfersView,
};
