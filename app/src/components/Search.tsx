import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import { cn } from "@/lib/cn";
import { isMod, searchShortcut } from "@/lib/platform";
import { useUsableModules } from "@/modules/registry";
import { modulesPageModule } from "@/modules/modules-page";
import { Icon, icons, type IconData } from "./Icon";
import { Kbd } from "./ui";

interface Item {
  id: string;
  label: string;
  hint?: string;
  icon: IconData;
  section: "Pages" | "Actions" | "Clipboard";
  haystack: string;
  run: () => void;
}

const spring = { type: "spring", duration: 0.4, bounce: 0.12 } as const;

/**
 * Search lives in the title bar, the way Windows 11 does it: a field that opens a
 * results flyout beneath it. It finds pages, actions and clipboard history.
 */
export function Search() {
  const open = useTether((s) => s.searchOpen);
  const setOpen = useTether((s) => s.setSearch);
  const { disabledModules, clips, devices, theme, paused } = useTether();
  const modules = useUsableModules();
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const items = useMemo<Item[]>(() => {
    const s = useTether.getState();
    const pages: Item[] = [...modules.filter((m) => !disabledModules.includes(m.id)), modulesPageModule].map((m) => ({
      id: `go-${m.id}`,
      label: m.label,
      hint: m.summary,
      icon: m.icon,
      section: "Pages",
      haystack: `${m.label} ${m.summary}`.toLowerCase(),
      run: () => s.setActiveModule(m.id),
    }));
    const commands: Item[] = [
      ...modules
        .filter((m) => !disabledModules.includes(m.id))
        .flatMap((m) => m.commands?.() ?? [])
        .map((c) => ({ id: c.id, label: c.label, icon: icons.flash, section: "Actions" as const, haystack: `${c.label} ${c.keywords ?? ""}`.toLowerCase(), run: c.run })),
      {
        id: "pause",
        label: paused ? "Resume sync" : "Pause sync",
        icon: paused ? icons.play : icons.pause,
        section: "Actions",
        haystack: "pause resume sync stop start clipboard",
        run: () => void s.setPaused(!paused).catch(() => s.toast("Could not change the pause setting")),
      },
      { id: "theme", label: theme === "dark" ? "Use light appearance" : "Use dark appearance", icon: theme === "dark" ? icons.sun : icons.moon, section: "Actions", haystack: "theme appearance dark light mode", run: s.toggleTheme },
    ];
    const names = new Map(devices.map((d) => [d.id, d.name]));
    const history: Item[] = clips
      .filter((c) => !c.sensitive)
      .slice(0, 50)
      .map((c) => ({
        id: `clip-${c.id}`,
        label: c.text.split("\n")[0],
        hint: names.get(c.device),
        icon: icons.copy,
        section: "Clipboard" as const,
        haystack: c.text.toLowerCase(),
        run: () =>
          void s.backend.copyHere(c).then(
            () => s.toast("Copied to this device"),
            () => s.toast("Could not copy"),
          ),
      }));
    return [...pages, ...commands, ...history];
  }, [disabledModules, clips, devices, theme, paused, modules]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [...items.filter((i) => i.section === "Pages"), ...items.filter((i) => i.section === "Clipboard").slice(0, 3)];
    return items.filter((i) => i.haystack.includes(q)).slice(0, 24);
  }, [items, query]);

  const active = Math.min(index, Math.max(0, results.length - 1));

  // Platform shortcut (Ctrl+F, or ⌘F on a Mac) and "/" focus the field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLElement && /^(input|textarea)$/i.test(e.target.tagName);
      if ((isMod(e) && e.key.toLowerCase() === "f") || (e.key === "/" && !typing)) {
        e.preventDefault();
        inputRef.current?.focus();
        setOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  // Click outside closes the flyout.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) {
        setOpen(false);
        setQuery("");
      }
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [open, setOpen]);

  useEffect(() => {
    listRef.current?.querySelector(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const choose = (i?: Item) => {
    if (!i) return;
    setOpen(false);
    setQuery("");
    inputRef.current?.blur();
    i.run();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setIndex(Math.min(results.length - 1, active + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setIndex(Math.max(0, active - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(results[active]);
    } else if (e.key === "Escape") {
      setOpen(false);
      setQuery("");
      inputRef.current?.blur();
    }
  };

  let lastSection = "";
  return (
    <div ref={rootRef} className={cn("relative transition-[width] duration-300 ease-out", open ? "w-[300px]" : "w-[200px] 2xl:w-[300px]")}>
      <label className={cn("glass flex h-10 items-center gap-2.5 rounded-full px-4 text-muted-foreground transition-[box-shadow] duration-300", open && "shadow-[0_0_0_1px_var(--ring)]")}>
        <Icon icon={icons.search} size={16} />
        <input
          ref={inputRef}
          value={query}
          onFocus={() => setOpen(true)}
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
            setOpen(true);
          }}
          onKeyDown={onKeyDown}
          placeholder="Search"
          aria-label="Search"
          className="selectable min-w-0 flex-1 bg-transparent text-[13.5px] text-foreground outline-none placeholder:text-subtle"
        />
        {!open && (
          <span className="hidden xl:inline-flex">
            <Kbd>{searchShortcut}</Kbd>
          </span>
        )}
      </label>

      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, y: -8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.985 }}
            transition={spring}
            style={{ transformOrigin: "top right" }}
            className="glass absolute top-[calc(100%+10px)] right-0 z-40 w-[440px] max-w-[calc(100vw-32px)] overflow-hidden rounded-3xl p-2 shadow-[var(--shadow-lift)]"
          >
            <div ref={listRef} className="max-h-[380px] overflow-y-auto">
              {results.length === 0 && <p className="px-4 py-8 text-center text-[13px] text-muted-foreground">Nothing matches “{query}”.</p>}
              {results.map((r, i) => {
                const header = r.section !== lastSection ? r.section : null;
                lastSection = r.section;
                return (
                  <div key={r.id}>
                    {header && <div className="px-3 pt-2.5 pb-1 text-[10.5px] font-medium tracking-[0.14em] text-subtle uppercase">{header}</div>}
                    <button
                      data-i={i}
                      onMouseMove={() => setIndex(i)}
                      onClick={() => choose(r)}
                      className={cn("flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left transition-colors duration-150", i === active && "bg-muted")}
                    >
                      <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
                        <Icon icon={r.icon} size={15} />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[13.5px] text-foreground">{r.label}</span>
                        {r.hint && <span className="block truncate text-xs text-subtle">{r.hint}</span>}
                      </span>
                    </button>
                  </div>
                );
              })}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
