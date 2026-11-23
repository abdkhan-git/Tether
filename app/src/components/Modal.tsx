import { useEffect, type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import { icons } from "./Icon";
import { IconButton } from "./ui";

export const spring = { type: "spring", duration: 0.5, bounce: 0.18 } as const;

export function Modal({ open, onClose, title, children, width = 440 }: { open: boolean; onClose: () => void; title: string; children: ReactNode; width?: number }) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-md"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          onMouseDown={(e) => e.target === e.currentTarget && onClose()}
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label={title}
            initial={{ opacity: 0, scale: 0.94, y: 16 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 6 }}
            transition={spring}
            style={{ width }}
            className="glass max-w-full rounded-[28px] p-7 shadow-[var(--shadow-lift)]"
          >
            <div className="mb-5 flex items-start justify-between gap-4">
              <h2 className="display text-[28px] text-foreground">{title}</h2>
              <IconButton icon={icons.close} label="Close" onClick={onClose} className="-mt-1 -mr-2" />
            </div>
            {children}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

