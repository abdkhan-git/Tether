import { useTether } from "@/data/store";
import SwipeToast from "./rb/SwipeToast";

/** Bottom-right, where Windows puts its own notifications. Swipe one away, or let the fuse burn down. */
export function Toasts() {
  const toasts = useTether((s) => s.toasts);
  const dismiss = useTether((s) => s.dismissToast);
  return (
    <div className="pointer-events-none fixed right-6 bottom-6 z-[60] flex w-[340px] max-w-[calc(100vw-32px)] flex-col gap-2 [&>*]:pointer-events-auto" aria-live="polite">
      {toasts.map((t) => (
        <SwipeToast
          key={t.id}
          inline
          open
          title={t.message}
          description={t.description}
          duration={3400}
          fuse="bottom"
          closeButton={false}
          width={340}
          radius={18}
          background="var(--surface-2)"
          color="var(--foreground)"
          fuseColor="var(--accent)"
          onClose={() => dismiss(t.id)}
        />
      ))}
    </div>
  );
}
