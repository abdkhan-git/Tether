import { useEffect } from "react";
import { AnimatePresence, MotionConfig, motion } from "motion/react";
import { useTether } from "@/data/store";
import { liveModules } from "@/modules/registry";
import { modulesPageModule } from "@/modules/modules-page";
import { PairDialog } from "@/components/PairDialog";
import { Toasts } from "@/components/Toasts";
import { TopBar } from "@/components/TopBar";
import { WarmTooltipGroup } from "@/components/ui";

export default function App() {
  const activeId = useTether((s) => s.activeModule);
  const disabled = useTether((s) => s.disabledModules);
  const theme = useTether((s) => s.theme);
  const setActive = useTether((s) => s.setActiveModule);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  // Follow the URL hash, so deep links and back/forward work.
  useEffect(() => {
    const onHash = () => {
      const id = location.hash.slice(1);
      if (id && id !== useTether.getState().activeModule) setActive(id);
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, [setActive]);

  // If the open module gets switched off, fall back to the overview.
  const active = [...liveModules, modulesPageModule].find((m) => m.id === activeId);
  const usable = !!active && (active.id === "modules" || !disabled.includes(active.id));
  useEffect(() => {
    if (!usable) setActive("overview");
  }, [usable, setActive]);

  const View = usable ? active?.view : undefined;

  return (
    <MotionConfig reducedMotion="user">
      <WarmTooltipGroup>
        <div className="flex h-full flex-col">
          <TopBar />
          <main className="relative min-h-0 flex-1">
            <AnimatePresence mode="wait" initial={false}>
              <motion.div
                key={activeId}
                className="absolute inset-0 overflow-y-auto"
                initial={{ opacity: 0, y: 10, filter: "blur(6px)" }}
                animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                exit={{ opacity: 0, y: -4, filter: "blur(4px)" }}
                transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
              >
                <div className="mx-auto w-full max-w-[1120px] px-6 pt-8 pb-16">{View && <View />}</div>
              </motion.div>
            </AnimatePresence>
          </main>
        </div>
        <PairDialog />
        <Toasts />
      </WarmTooltipGroup>
    </MotionConfig>
  );
}
