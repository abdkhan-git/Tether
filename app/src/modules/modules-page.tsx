import { useTether } from "@/data/store";
import { cn } from "@/lib/cn";
import { Icon, icons } from "@/components/Icon";
import { Card, Chip, PageHeader, Panel, SectionLabel, Switch } from "@/components/ui";
import { plannedModules } from "./planned";
import { liveModules } from "./registry";
import type { TetherModule } from "./types";

function Row({ m, enabled, onToggle }: { m: TetherModule; enabled?: boolean; onToggle?: () => void }) {
  return (
    <div className={cn("flex items-center gap-4 px-5 py-4", m.status === "planned" && "opacity-75")}>
      <span className="flex size-10 shrink-0 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
        <Icon icon={m.icon} size={18} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[14.5px] font-medium">{m.label}</span>
          {m.status === "preview" && <Chip>Preview</Chip>}
          {m.status === "planned" && <Chip>{m.note}</Chip>}
        </div>
        <p className="mt-0.5 text-[13px] text-muted-foreground">{m.summary}</p>
      </div>
      {onToggle && (m.core ? <span className="text-xs text-subtle">Always on</span> : <Switch checked={!!enabled} onChange={onToggle} label={`Show ${m.label}`} />)}
    </div>
  );
}

function ModulesView() {
  const { disabledModules, toggleModule } = useTether();
  return (
    <div>
      <PageHeader eyebrow="Modules" title="Make it yours." subtitle="Tether is a set of features on one synced, encrypted core. Turn on what you use; new modules arrive here." />
      <div className="grid gap-10 lg:grid-cols-[minmax(0,1.25fr)_minmax(0,1fr)]">
        <section>
          <SectionLabel>Installed</SectionLabel>
          <Panel className="divide-y divide-border">
            {liveModules.map((m) => (
              <Row key={m.id} m={m} enabled={!disabledModules.includes(m.id)} onToggle={() => toggleModule(m.id)} />
            ))}
          </Panel>
        </section>
        <section>
          <SectionLabel>Coming</SectionLabel>
          <Panel className="divide-y divide-border">
            {plannedModules.map((m) => (
              <Row key={m.id} m={m} />
            ))}
          </Panel>
          <Card pad={22} className="mt-4">
            <p className="text-[13.5px] text-muted-foreground">
              Every module reads and writes the same replicated log, so each one is offline-ready, end-to-end encrypted and checked by the simulator from day one.
            </p>
          </Card>
        </section>
      </div>
    </div>
  );
}

export const modulesPageModule: TetherModule = {
  id: "modules",
  label: "Modules",
  icon: icons.grid,
  group: "Network",
  summary: "Turn features on and off.",
  status: "stable",
  core: true,
  view: ModulesView,
};
