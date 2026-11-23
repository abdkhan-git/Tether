import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import type { Workspace } from "@/data/types";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { Icon, icons, type IconData } from "@/components/Icon";
import { Button, Card, DeviceGlyph, PageHeader, PrimaryAction } from "@/components/ui";
import type { TetherModule } from "./types";

const spring = { type: "spring", duration: 0.5, bounce: 0.15 } as const;
const count = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

function Group({ icon, label, items }: { icon: IconData; label: string; items: string[] }) {
  if (!items.length) return null;
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium tracking-[0.12em] text-subtle uppercase">
        <Icon icon={icon} size={13} />
        {label} · {items.length}
      </div>
      <ul className="flex flex-col gap-1">
        {items.map((i) => (
          <li key={i} className="selectable truncate font-mono text-xs text-muted-foreground">
            {i}
          </li>
        ))}
      </ul>
    </div>
  );
}

function WorkspaceCard({ w }: { w: Workspace }) {
  const { devices, me, toast } = useTether();
  const [open, setOpen] = useState(false);
  const dev = devices.find((d) => d.id === w.device);
  const here = w.device === me;

  return (
    <motion.li layout="position" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={spring}>
      <Card pad={0}>
        <div className="flex flex-col gap-5 p-6">
          <div className="flex items-start gap-3.5">
            {dev && <DeviceGlyph platform={dev.platform} size={42} online={dev.connection !== "offline"} />}
            <div className="min-w-0 flex-1">
              <h2 className="truncate text-[17px] font-medium tracking-tight">{w.name}</h2>
              <p className="mt-0.5 text-[13px] text-muted-foreground">
                {here ? "This device" : dev?.name} · {timeAgo(w.ts)}
              </p>
            </div>
          </div>
          <p className="text-[13px] text-muted-foreground">
            {count(w.tabs.length, "tab")} · {count(w.files.length, "file")} · {count(w.terminals.length, "terminal")}
          </p>
          <div className="flex items-center justify-between">
            <Button variant={here ? "outline" : "solid"} icon={icons.play} onClick={() => toast(`Restoring ${count(w.tabs.length, "tab")}, ${count(w.files.length, "file")}, ${count(w.terminals.length, "terminal")}`)}>
              {here ? "Reopen" : "Resume here"}
            </Button>
            <button onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-label="Show contents" className="flex size-9 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground">
              <Icon icon={icons.chevronDown} size={17} className={cn("transition-transform duration-300", open && "rotate-180")} />
            </button>
          </div>
        </div>
        <AnimatePresence initial={false}>
          {open && (
            <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={spring} className="overflow-hidden">
              <div className="flex flex-col gap-4 border-t border-border p-6">
                <Group icon={icons.globe} label="Tabs" items={w.tabs.map((t) => t.title)} />
                <Group icon={icons.file} label="Files" items={w.files} />
                <Group icon={icons.code} label="Terminals" items={w.terminals} />
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </Card>
    </motion.li>
  );
}

function WorkspacesView() {
  const { workspaces, captureWorkspace, toast } = useTether();
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");

  const commit = () => {
    const n = name.trim() || `Workspace ${new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
    captureWorkspace(n);
    toast(`Captured “${n}”`);
    setName("");
    setNaming(false);
  };

  return (
    <div>
      <PageHeader
        eyebrow="Workspaces"
        title="Pick up where you left off."
        subtitle="Snapshot your tabs, files and terminals, then resume them on any device with one click."
        actions={
          naming ? (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                commit();
              }}
              className="flex items-center gap-2"
            >
              <input autoFocus value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setNaming(false)} placeholder="Name this workspace" className="selectable glass h-10 w-56 rounded-full px-4 text-[13.5px] outline-none placeholder:text-subtle focus:shadow-[0_0_0_1px_var(--ring)]" />
              <Button variant="solid" type="submit">
                Capture
              </Button>
            </form>
          ) : (
            <PrimaryAction icon={icons.add} onClick={() => setNaming(true)}>
              Capture workspace
            </PrimaryAction>
          )
        }
      />
      <ul className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {workspaces.map((w) => (
          <WorkspaceCard key={w.id} w={w} />
        ))}
      </ul>
      <p className="mt-8 px-1 text-xs text-subtle">Tab capture needs the browser extension (Stage 4). These workspaces are preview data.</p>
    </div>
  );
}

export const workspacesModule: TetherModule = {
  id: "workspaces",
  label: "Workspaces",
  icon: icons.browser,
  group: "Workspace",
  summary: "Snapshot your tabs, files and terminals, then resume on any device.",
  status: "preview",
  note: "Roadmap · P6",
  view: WorkspacesView,
};
