import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import type { Clip, Device } from "@/data/types";
import { contentOf, type ContentKind } from "@/lib/content/detect";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { os, thisDevice } from "@/lib/platform";
import { Icon, icons, type IconData } from "@/components/Icon";
import { Button, Card, Chip, DeviceGlyph, EmptyState, IconButton, PageHeader, Panel, PrimaryAction, Segmented } from "@/components/ui";
import type { TetherModule } from "../types";
import { Preview, SecretGate, useCopy } from "./previews";

export const kindIcon: Record<ContentKind, IconData> = {
  json: icons.braces,
  code: icons.code,
  link: icons.link,
  color: icons.palette,
  jwt: icons.key,
  secret: icons.lock,
  time: icons.clock,
  table: icons.table,
  path: icons.folder,
  email: icons.mail,
  math: icons.calculator,
  uuid: icons.id,
  text: icons.text,
};

const spring = { type: "spring", duration: 0.45, bounce: 0.12 } as const;

type Filter = "all" | "pinned" | "mine";

function firstLine(c: Clip) {
  if (c.sensitive) return "•••••••••••••";
  return c.text.trim().split("\n")[0];
}

function ClipRow({ clip, device, active, me, onSelect, onCopy }: { clip: Clip; device?: Device; active: boolean; me: string; onSelect: () => void; onCopy: () => void }) {
  const content = contentOf(clip.text);
  const mono = !clip.sensitive && (content.kind === "code" || content.kind === "json" || content.kind === "uuid");
  return (
    <button onClick={onSelect} onDoubleClick={onCopy} className="relative flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left">
      {active && <motion.span layoutId="clip-selected" className="absolute inset-0 rounded-2xl bg-muted" transition={spring} />}
      <span className="relative flex size-9 shrink-0 items-center justify-center rounded-xl border border-border bg-surface-2 text-muted-foreground">
        {content.kind === "color" && !clip.sensitive ? <span className="size-4 rounded-full border border-border" style={{ background: clip.text.trim() }} /> : <Icon icon={kindIcon[content.kind]} size={16} />}
      </span>
      <span className="relative min-w-0 flex-1">
        <span className={cn("block truncate text-[13.5px]", mono && "font-mono text-[12.5px]")}>{firstLine(clip)}</span>
        <span className="mt-0.5 flex items-center gap-1.5 text-xs text-subtle">
          <span className="truncate">
            {content.label} · {clip.device === me ? "This device" : device?.name} · {timeAgo(clip.ts)}
          </span>
          {clip.pinned && <Icon icon={icons.pin} size={11} className="shrink-0 text-muted-foreground" />}
          {clip.fresh && <span className="shrink-0 rounded-full bg-accent-soft px-1.5 text-[10px] font-medium text-accent">New</span>}
        </span>
      </span>
    </button>
  );
}

function Detail({ clip, device, peers }: { clip: Clip; device?: Device; peers: Device[] }) {
  const { updateClip, deleteClip, toast } = useTether();
  const copy = useCopy();
  const [revealed, setRevealed] = useState(false);
  const content = contentOf(clip.text);
  const hidden = clip.sensitive && !revealed;

  return (
    <Card pad={28} className="flex min-h-[360px] flex-col gap-6">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <Chip icon={<Icon icon={kindIcon[content.kind]} size={13} />} className="text-foreground">
          {content.label}
        </Chip>
        <span className="text-[13px] text-muted-foreground">{content.summary}</span>
        <span className="ml-auto flex items-center gap-2 text-[13px] text-muted-foreground">
          {device && <DeviceGlyph platform={device.platform} size={24} />}
          {device?.isMe ? "This device" : device?.name} · {timeAgo(clip.ts)}
        </span>
      </div>

      <AnimatePresence mode="wait" initial={false}>
        <motion.div key={`${clip.id}:${hidden}`} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }} transition={{ duration: 0.18 }} className="min-w-0 flex-1">
          {hidden ? <SecretGate noun={content.kind === "secret" ? content.secret.noun : content.kind === "jwt" ? "an access token" : "something you marked private"} text={clip.text} onReveal={() => setRevealed(true)} /> : <Preview content={content} text={clip.text} />}
        </motion.div>
      </AnimatePresence>

      <div className="flex flex-wrap items-center gap-1.5 border-t border-border pt-5">
        <PrimaryAction icon={icons.copy} onClick={() => copy(clip.text, `Copied to ${thisDevice[os]}`)}>
          Copy to {thisDevice[os]}
        </PrimaryAction>
        <IconButton icon={clip.pinned ? icons.pinOff : icons.pin} label={clip.pinned ? "Unpin" : "Pin"} onClick={() => updateClip(clip.id, { pinned: !clip.pinned })} />
        <IconButton
          icon={icons.lock}
          label={clip.sensitive ? "Allow syncing" : "Keep on this device"}
          onClick={() => {
            updateClip(clip.id, { sensitive: !clip.sensitive });
            setRevealed(false);
          }}
        />
        {!clip.sensitive && peers.length > 0 && (
          <div className="ml-2 flex flex-wrap items-center gap-1.5">
            {peers.map((p) => (
              <Button key={p.id} variant="ghost" size="sm" icon={icons.send} onClick={() => toast(`Pushed to ${p.name}`)}>
                {p.name}
              </Button>
            ))}
          </div>
        )}
        <div className="ml-auto">
          <IconButton
            icon={icons.trash}
            label="Delete everywhere"
            tone="danger"
            onClick={() => {
              deleteClip(clip.id);
              toast("Removed from every device");
            }}
          />
        </div>
      </div>
    </Card>
  );
}

function ClipboardView() {
  const { clips, devices, me, updateClip } = useTether();
  const copy = useCopy();
  const [filter, setFilter] = useState<Filter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const deviceById = useMemo(() => new Map(devices.map((d) => [d.id, d])), [devices]);
  const visible = useMemo(() => clips.filter((c) => (filter === "pinned" ? c.pinned : filter === "mine" ? c.device === me : true)).sort((a, b) => b.ts - a.ts), [clips, filter, me]);
  const selected = visible.find((c) => c.id === selectedId) ?? visible[0] ?? null;
  const peers = devices.filter((d) => !d.isMe && d.connection !== "offline");

  // The "New" marker fades after a moment.
  useEffect(() => {
    const fresh = clips.filter((c) => c.fresh);
    if (!fresh.length) return;
    const t = setTimeout(() => fresh.forEach((c) => updateClip(c.id, { fresh: false })), 2400);
    return () => clearTimeout(t);
  }, [clips, updateClip]);

  const copyHere = (c: Clip) => copy(c.text, `Copied to ${thisDevice[os]}`);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!visible.length) return;
    const i = Math.max(0, visible.findIndex((c) => c.id === selected?.id));
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = visible[Math.min(visible.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))];
      setSelectedId(next.id);
      listRef.current?.querySelector(`[data-id="${next.id}"]`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter" && selected) {
      copyHere(selected);
    }
  };

  return (
    <div>
      <PageHeader
        eyebrow="Clipboard"
        title="Everything you've copied"
        subtitle="Tether recognises what you copy, formats it, and shows it the way it should be read."
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

      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,360px)_minmax(0,1fr)]">
        <Panel className="max-h-[calc(100vh-300px)] min-h-[360px] overflow-y-auto p-2">
          {visible.length === 0 ? (
            <EmptyState title="Nothing here" hint="Copy something on any paired device and it appears here." />
          ) : (
            <ul ref={listRef} tabIndex={0} onKeyDown={onKeyDown} className="flex flex-col gap-0.5 outline-none" aria-label="Clipboard history">
              <AnimatePresence initial={false}>
                {visible.map((c) => (
                  <motion.li key={c.id} data-id={c.id} layout="position" initial={{ opacity: 0, y: -10 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, height: 0 }} transition={spring}>
                    <ClipRow clip={c} device={deviceById.get(c.device)} active={selected?.id === c.id} me={me} onSelect={() => setSelectedId(c.id)} onCopy={() => copyHere(c)} />
                  </motion.li>
                ))}
              </AnimatePresence>
            </ul>
          )}
        </Panel>

        {selected ? (
          <Detail key={selected.id} clip={selected} device={deviceById.get(selected.device)} peers={peers} />
        ) : (
          <Card className="min-h-[360px]">
            <EmptyState title="Select an item" hint="Double-click an item, or press Enter, to copy it here." />
          </Card>
        )}
      </div>
    </div>
  );
}

export const clipboardModule: TetherModule = {
  id: "clipboard",
  label: "Clipboard",
  icon: icons.clipboard,
  group: "Sync",
  summary: "Everything you copy, recognised, formatted and synced.",
  status: "stable",
  core: true,
  view: ClipboardView,
};
