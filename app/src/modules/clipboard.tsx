import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import type { Clip, ClipKind, Device } from "@/data/types";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { os, thisDevice } from "@/lib/platform";
import { Icon, icons, type IconData } from "@/components/Icon";
import { Button, Card, Chip, DeviceGlyph, EmptyState, IconButton, Panel, PageHeader, PrimaryAction, Segmented } from "@/components/ui";
import type { TetherModule } from "./types";

const kindIcon: Record<ClipKind, IconData> = { text: icons.file, link: icons.link, code: icons.code };
const spring = { type: "spring", duration: 0.5, bounce: 0.15 } as const;

type Filter = "all" | "pinned" | "mine";

function preview(c: Clip) {
  return c.sensitive ? "••••••••••••" : c.text.split("\n")[0];
}

function ClipboardView() {
  const { clips, devices, me, updateClip, deleteClip, toast } = useTether();
  const [filter, setFilter] = useState<Filter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<string | null>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const deviceById = useMemo(() => new Map(devices.map((d) => [d.id, d])), [devices]);
  const visible = useMemo(
    () =>
      clips
        .filter((c) => (filter === "pinned" ? c.pinned : filter === "mine" ? c.device === me : true))
        .sort((a, b) => b.ts - a.ts),
    [clips, filter, me],
  );
  const selected = visible.find((c) => c.id === selectedId) ?? visible[0] ?? null;

  // The "new" highlight fades after a moment.
  useEffect(() => {
    const fresh = clips.filter((c) => c.fresh);
    if (!fresh.length) return;
    const t = setTimeout(() => fresh.forEach((c) => updateClip(c.id, { fresh: false })), 2400);
    return () => clearTimeout(t);
  }, [clips, updateClip]);

  const copyHere = async (c: Clip) => {
    try {
      await navigator.clipboard.writeText(c.text);
      toast(`Copied to ${thisDevice[os]}`);
    } catch {
      toast("Clipboard access was blocked");
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!visible.length) return;
    const i = Math.max(0, visible.findIndex((c) => c.id === selected?.id));
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = visible[Math.min(visible.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))];
      setSelectedId(next.id);
      listRef.current?.querySelector(`[data-id="${next.id}"]`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter" && selected) {
      void copyHere(selected);
    }
  };

  const peers = devices.filter((d) => !d.isMe && d.connection !== "offline");

  return (
    <div>
      <PageHeader
        eyebrow="Clipboard"
        title="Everything you've copied."
        subtitle={`${clips.length} items from ${devices.filter((d) => d.connection !== "offline").length} connected devices, searchable and always in sync.`}
        actions={
          <Segmented
            label="Filter"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "All" },
              { value: "pinned", label: "Pinned" },
              { value: "mine", label: "This device" },
            ]}
          />
        }
      />

      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,380px)_minmax(0,1fr)]">
        <Panel className="max-h-[calc(100vh-330px)] min-h-[320px] overflow-y-auto p-2">
          {visible.length === 0 ? (
            <EmptyState title="Nothing here" hint="Copy something on any paired device and it appears here." />
          ) : (
            <ul ref={listRef} tabIndex={0} onKeyDown={onKeyDown} className="flex flex-col gap-0.5 outline-none" aria-label="Clipboard history">
              <AnimatePresence initial={false}>
                {visible.map((c) => {
                  const dev = deviceById.get(c.device);
                  const active = selected?.id === c.id;
                  return (
                    <motion.li key={c.id} data-id={c.id} layout="position" initial={{ opacity: 0, y: -12 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, height: 0 }} transition={spring}>
                      <button onClick={() => setSelectedId(c.id)} onDoubleClick={() => void copyHere(c)} className="relative flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left">
                        {active && <motion.span layoutId="clip-selected" className="absolute inset-0 rounded-2xl bg-muted" transition={spring} />}
                        {active && <motion.span layoutId="clip-bar" className="absolute top-3 bottom-3 left-0 w-[3px] rounded-full bg-accent" transition={spring} />}
                        {dev && <DeviceGlyph platform={dev.platform} size={34} online={dev.connection !== "offline"} />}
                        <span className="relative min-w-0 flex-1">
                          <span className={cn("block truncate text-[13.5px]", c.kind === "code" && !c.sensitive && "font-mono text-xs")}>{preview(c)}</span>
                          <span className="mt-0.5 flex items-center gap-1.5 text-xs text-subtle">
                            {c.device === me ? "This device" : dev?.name} · {timeAgo(c.ts)}
                            {c.pinned && <Icon icon={icons.pin} size={11} className="text-accent" />}
                            {c.sensitive && <Icon icon={icons.lock} size={11} />}
                            {c.fresh && <span className="rounded-full bg-accent-soft px-1.5 text-[10px] font-medium text-accent">New</span>}
                          </span>
                        </span>
                      </button>
                    </motion.li>
                  );
                })}
              </AnimatePresence>
            </ul>
          )}
        </Panel>

        {selected ? (
          <Detail
            clip={selected}
            device={deviceById.get(selected.device)}
            peers={peers}
            revealed={revealed === selected.id}
            onReveal={() => setRevealed(selected.id)}
            onCopy={() => void copyHere(selected)}
            onPin={() => updateClip(selected.id, { pinned: !selected.pinned })}
            onSensitive={() => {
              updateClip(selected.id, { sensitive: !selected.sensitive });
              setRevealed(null);
            }}
            onDelete={() => {
              deleteClip(selected.id);
              toast("Removed from every device");
            }}
            onSend={(d) => toast(`Pushed to ${d.name}`)}
          />
        ) : (
          <Card className="min-h-[320px]">
            <EmptyState title="Select an item" hint="Double-click an item, or press Enter, to copy it here." />
          </Card>
        )}
      </div>
    </div>
  );
}

function Detail({
  clip,
  device,
  peers,
  revealed,
  onReveal,
  onCopy,
  onPin,
  onSensitive,
  onDelete,
  onSend,
}: {
  clip: Clip;
  device?: Device;
  peers: Device[];
  revealed: boolean;
  onReveal: () => void;
  onCopy: () => void;
  onPin: () => void;
  onSensitive: () => void;
  onDelete: () => void;
  onSend: (d: Device) => void;
}) {
  const hidden = clip.sensitive && !revealed;
  return (
    <Card pad={28} className="flex min-h-[320px] flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        {device && <DeviceGlyph platform={device.platform} size={30} />}
        <span className="text-[13px] font-medium">{device?.isMe ? "This device" : device?.name}</span>
        <span className="text-subtle">·</span>
        <span className="text-[13px] text-muted-foreground">{timeAgo(clip.ts)}</span>
        <Chip className="ml-1 capitalize" icon={<Icon icon={kindIcon[clip.kind]} size={12} />}>
          {clip.kind}
        </Chip>
        {clip.sensitive && (
          <Chip className="bg-accent-soft text-accent" icon={<Icon icon={icons.lock} size={12} />}>
            Stays on this device
          </Chip>
        )}
      </div>

      <AnimatePresence mode="wait" initial={false}>
        <motion.div key={clip.id} initial={{ opacity: 0, y: 8, filter: "blur(4px)" }} animate={{ opacity: 1, y: 0, filter: "blur(0px)" }} exit={{ opacity: 0 }} transition={{ duration: 0.22 }} className="relative flex-1">
          <pre className={cn("selectable max-h-[40vh] overflow-auto break-words whitespace-pre-wrap transition-[filter] duration-300", clip.kind === "code" ? "font-mono text-[13px] leading-relaxed" : "font-sans text-[17px] leading-relaxed text-foreground", hidden && "pointer-events-none blur-lg select-none")}>{clip.text}</pre>
          {hidden && (
            <div className="absolute inset-0 flex items-center justify-center">
              <Button variant="outline" icon={icons.hide} onClick={onReveal} className="bg-background/60 backdrop-blur-md">
                Reveal
              </Button>
            </div>
          )}
        </motion.div>
      </AnimatePresence>

      <div className="flex flex-wrap items-center gap-1.5 border-t border-border pt-5">
        <PrimaryAction icon={icons.copy} onClick={onCopy}>
          Copy to {thisDevice[os]}
        </PrimaryAction>
        <IconButton icon={clip.pinned ? icons.pinOff : icons.pin} label={clip.pinned ? "Unpin" : "Pin"} onClick={onPin} />
        <IconButton icon={icons.lock} label={clip.sensitive ? "Allow syncing" : "Keep on this device"} onClick={onSensitive} />
        <div className="ml-auto">
          <IconButton icon={icons.trash} label="Delete everywhere" tone="danger" onClick={onDelete} />
        </div>
      </div>

      {!clip.sensitive && peers.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="mr-1 text-xs text-subtle">Push to</span>
          {peers.map((p) => (
            <Button key={p.id} variant="outline" size="sm" icon={icons.send} onClick={() => onSend(p)}>
              {p.name}
            </Button>
          ))}
        </div>
      )}
      {clip.sensitive && <p className="text-xs text-subtle">Sensitive items never leave the device that copied them.</p>}
    </Card>
  );
}

export const clipboardModule: TetherModule = {
  id: "clipboard",
  label: "Clipboard",
  icon: icons.clipboard,
  group: "Sync",
  summary: "Searchable history of everything you copy, on every device.",
  status: "stable",
  core: true,
  view: ClipboardView,
};
