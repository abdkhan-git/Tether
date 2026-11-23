import { useMemo } from "react";
import { useTether } from "@/data/store";
import { liveModules } from "@/modules/registry";
import PillNav from "./rb/PillNav";
import { Icon, icons, platformGlyph } from "./Icon";
import { Search } from "./Search";
import WarmTooltip from "./rb/WarmTooltip";
import { IconButton, StatusDot, palette } from "./ui";

/** Two linked rings: the tether. */
function Mark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={className} fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" aria-hidden>
      <circle cx="11.5" cy="16" r="7.5" />
      <circle cx="20.5" cy="16" r="7.5" opacity="0.55" />
    </svg>
  );
}

function Presence() {
  const devices = useTether((s) => s.devices);
  const setActive = useTether((s) => s.setActiveModule);
  const peers = devices.filter((d) => !d.isMe);
  const online = peers.filter((d) => d.connection !== "offline").length;
  return (
    <WarmTooltip content={`${online} of ${peers.length} devices connected`} side="bottom" size="sm">
      <button onClick={() => setActive("devices")} aria-label="Devices" className="group flex items-center gap-2.5 rounded-full py-1 pr-1 pl-3 transition-colors hover:bg-muted">
        <span className="hidden text-xs whitespace-nowrap text-muted-foreground transition-colors group-hover:text-foreground 2xl:inline">
          {online} connected
        </span>
        <span className="flex -space-x-2">
          {devices.map((d) => (
            <span key={d.id} className="relative flex size-8 items-center justify-center rounded-full border-2 border-background bg-surface-2 text-foreground" style={{ opacity: d.connection === "offline" ? 0.45 : 1 }}>
              <Icon icon={platformGlyph[d.platform]} size={14} />
              {d.connection !== "offline" && <StatusDot connection={d.connection} className="absolute -right-px -bottom-px scale-[0.8]" />}
            </span>
          ))}
        </span>
      </button>
    </WarmTooltip>
  );
}

export function TopBar() {
  const { activeModule, disabledModules, setActiveModule, theme, toggleTheme } = useTether();
  const c = palette[theme];

  const nav = useMemo(() => liveModules.filter((m) => !disabledModules.includes(m.id)), [disabledModules]);
  const items = nav.map((m) => ({ label: m.label, href: `#${m.id}`, ariaLabel: m.label }));

  return (
    <header className="relative z-30 grid shrink-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-4 px-6 pt-5 pb-2 2xl:grid-cols-[1fr_auto_1fr]" data-tauri-drag-region>
      <button onClick={() => setActiveModule("overview")} className="flex items-center gap-2.5 justify-self-start rounded-full py-1 pr-3 text-foreground transition-opacity hover:opacity-80" aria-label="Tether overview">
        <Mark className="size-7 text-accent" />
        <span className="display hidden text-[26px] leading-none 2xl:inline">Tether</span>
      </button>

      <div className="flex min-w-0 justify-center justify-self-center">
        <PillNav
          key={theme}
          theme={theme}
          layout="joined"
          size="md"
          surface="glass"
          itemStyle="plain"
          hoverEffect="glide"
          indicator="pill"
          highlightColor={c.accent}
          highlightTextColor={c.accentInk}
          items={items}
          activeHref={`#${activeModule}`}
          onItemClick={(item, _i, e) => {
            e.preventDefault();
            setActiveModule(item.href!.slice(1));
          }}
          ariaLabel="Primary"
        />
      </div>

      <div className="flex items-center justify-end gap-1.5 justify-self-end">
        <Search />
        <Presence />
        <IconButton icon={icons.grid} label="Modules" onClick={() => setActiveModule("modules")} />
        <IconButton icon={theme === "dark" ? icons.sun : icons.moon} label={theme === "dark" ? "Light appearance" : "Dark appearance"} onClick={toggleTheme} />
      </div>
    </header>
  );
}
