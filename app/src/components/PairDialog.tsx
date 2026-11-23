import { useMemo, useState } from "react";
import { motion } from "motion/react";
import { useTether } from "@/data/store";
import CodeSlots from "./rb/CodeSlots";
import { Modal } from "./Modal";
import { Segmented, useColors } from "./ui";

function randomCode() {
  const n = crypto.getRandomValues(new Uint32Array(1))[0] % 1_000_000;
  return n.toString().padStart(6, "0");
}

function Body({ onClose }: { onClose: () => void }) {
  const toast = useTether((s) => s.toast);
  const { c } = useColors();
  const code = useMemo(randomCode, []);
  const [mode, setMode] = useState<"show" | "enter">("show");
  const slotProps = { accentColor: c.accent, inkColor: c.ink, slotColor: c.track, digitColor: c.ink, dangerColor: c.danger, slotSize: 48, gap: 8, radius: 14 };

  return (
    <div className="flex flex-col gap-6">
      <p className="-mt-3 text-[13.5px] text-muted-foreground">
        {mode === "show" ? "Open Tether on the other device and type this code. It works once and expires in two minutes." : "Type the six-digit code that the other device is showing."}
      </p>
      <div className="flex justify-center">
        <Segmented
          label="Pairing mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: "show", label: "Show a code" },
            { value: "enter", label: "Enter a code" },
          ]}
        />
      </div>
      <div className="flex min-h-[64px] justify-center">
        {mode === "show" ? (
          <div className="flex gap-2" aria-label={`Pairing code ${code}`}>
            {code.split("").map((d, i) => (
              <motion.span
                key={i}
                initial={{ opacity: 0, y: 14, filter: "blur(6px)" }}
                animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                transition={{ type: "spring", duration: 0.6, bounce: 0.25, delay: 0.05 * i }}
                className={`flex h-16 w-12 items-center justify-center rounded-2xl border border-border bg-surface-2 font-mono text-[28px] font-medium text-foreground ${i === 3 ? "ml-3" : ""}`}
              >
                {d}
              </motion.span>
            ))}
          </div>
        ) : (
          <CodeSlots
            key="enter"
            length={6}
            autoFocus
            {...slotProps}
            onComplete={() => {
              toast("Pairing isn't wired up yet", "The daemon's Noise handshake is the next milestone.");
              onClose();
            }}
          />
        )}
      </div>
      <p className="text-center text-[11.5px] text-subtle">Preview: this code is random and not yet linked to the daemon.</p>
    </div>
  );
}

export function PairDialog() {
  const open = useTether((s) => s.pairOpen);
  const setOpen = useTether((s) => s.setPairOpen);
  return (
    <Modal open={open} onClose={() => setOpen(false)} title="Pair a device" width={460}>
      {open && <Body onClose={() => setOpen(false)} />}
    </Modal>
  );
}
