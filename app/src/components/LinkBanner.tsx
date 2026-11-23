import { AnimatePresence, motion } from "motion/react";
import { useTether } from "@/data/store";
import { Icon, icons } from "./Icon";

/**
 * A quiet strip under the title bar when the app cannot reach tetherd, or
 * when sync is paused. Nothing shows while all is well.
 */
export function LinkBanner() {
  const link = useTether((s) => s.link);
  const paused = useTether((s) => s.paused);
  const setPaused = useTether((s) => s.setPaused);
  const toast = useTether((s) => s.toast);

  const message =
    link.state === "unreachable"
      ? { key: "down", icon: icons.alert, text: "Can't reach the Tether service. Retrying…", detail: link.reason }
      : link.state === "connecting"
        ? { key: "connecting", icon: icons.wifi, text: "Connecting to the Tether service…" }
        : paused
          ? { key: "paused", icon: icons.pause, text: "Sync is paused. Nothing you copy here leaves this device, and copies from your other devices only reach history." }
          : null;

  return (
    <AnimatePresence initial={false}>
      {message && (
        <motion.div
          key={message.key}
          initial={{ opacity: 0, height: 0 }}
          animate={{ opacity: 1, height: "auto" }}
          exit={{ opacity: 0, height: 0 }}
          transition={{ type: "spring", duration: 0.45, bounce: 0 }}
          className="shrink-0 overflow-hidden px-6"
          role="status"
        >
          <div className="mx-auto mt-2 flex max-w-[1120px] items-center gap-3 rounded-2xl border border-border bg-surface px-4 py-2.5 text-[13px] text-muted-foreground">
            <Icon icon={message.icon} size={15} className={message.key === "down" ? "text-destructive" : undefined} />
            <span className="min-w-0 flex-1">
              {message.text}
              {"detail" in message && message.detail && <span className="ml-2 font-mono text-[11.5px] text-subtle">{message.detail}</span>}
            </span>
            {message.key === "paused" && (
              <button
                onClick={() => setPaused(false).catch(() => toast("Could not resume sync"))}
                className="shrink-0 rounded-full px-3 py-1 text-xs font-medium text-foreground transition-colors hover:bg-muted"
              >
                Resume
              </button>
            )}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
