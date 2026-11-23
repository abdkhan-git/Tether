import { useEffect, useRef, useState } from "react";
import { motion } from "motion/react";
import { errorText } from "@/data/daemon";
import { useTether } from "@/data/store";
import type { PairCode } from "@/data/types";
import CodeSlots from "./rb/CodeSlots";
import { Modal } from "./Modal";
import { Button, Segmented, useColors } from "./ui";

type ShowState = { phase: "opening" } | { phase: "waiting"; pair: PairCode; until: number } | { phase: "expired" } | { phase: "failed"; error: string };

function useSecondsLeft(until: number | null) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (until === null) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [until]);
  return until === null ? 0 : Math.max(0, Math.ceil((until - now) / 1000));
}

/** This device shows a code; the other device enters it. Done when a new member appears. */
function ShowCode({ onClose }: { onClose: () => void }) {
  const { backend, toast, port } = useTether();
  const devices = useTether((s) => s.devices);
  const members = useRef<Set<string> | null>(null);
  const [state, setState] = useState<ShowState>({ phase: "opening" });
  const left = useSecondsLeft(state.phase === "waiting" ? state.until : null);

  const open = () => {
    // Who is in the group now, so the newcomer stands out.
    members.current = new Set(useTether.getState().devices.map((d) => d.id));
    setState({ phase: "opening" });
    backend.pair().then(
      (pair) => setState({ phase: "waiting", pair, until: Date.now() + pair.validSecs * 1000 }),
      (e) => setState({ phase: "failed", error: errorText(e) }),
    );
  };
  useEffect(open, [backend]);

  useEffect(() => {
    if (state.phase === "waiting" && left === 0) setState({ phase: "expired" });
  }, [state.phase, left]);

  useEffect(() => {
    if (state.phase !== "waiting" || !members.current) return;
    const joined = devices.find((d) => !members.current!.has(d.id));
    if (joined) {
      toast(`Paired with ${joined.name}`, `Check that its fingerprint, ${joined.fingerprint}, matches what it shows for itself.`);
      onClose();
    }
  }, [devices, state.phase, toast, onClose]);

  if (state.phase === "failed" || state.phase === "expired") {
    return (
      <div className="flex min-h-[120px] flex-col items-center justify-center gap-4 text-center">
        <p className="text-[13.5px] text-muted-foreground">{state.phase === "expired" ? "That code expired before a device used it." : `Could not open pairing: ${state.error}`}</p>
        <Button variant="outline" size="sm" onClick={open}>
          Show a new code
        </Button>
      </div>
    );
  }

  const code = state.phase === "waiting" ? state.pair.code : "······";
  return (
    <div className="flex flex-col items-center gap-5">
      <div className="flex gap-2" aria-label={`Pairing code ${code}`}>
        {code.split("").map((d, i) => (
          <motion.span
            key={`${code}-${i}`}
            initial={{ opacity: 0, y: 14, filter: "blur(6px)" }}
            animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
            transition={{ type: "spring", duration: 0.6, bounce: 0.25, delay: 0.05 * i }}
            className={`flex h-16 w-12 items-center justify-center rounded-2xl border border-border bg-surface-2 font-mono text-[28px] font-medium text-foreground ${i === 3 ? "ml-3" : ""}`}
          >
            {d}
          </motion.span>
        ))}
      </div>
      <p className="text-center text-[12px] text-subtle">
        {state.phase === "waiting"
          ? `Waiting for the other device · ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")} left · ${state.pair.attempts} tries`
          : "Opening a pairing window…"}
        {port !== null && (
          <>
            <br />
            If the other device can't find this one, give it this device's IP address and port {port}.
          </>
        )}
      </p>
    </div>
  );
}

/** The other device shows a code; this one enters it and joins. */
function EnterCode({ onClose }: { onClose: () => void }) {
  const { backend, toast } = useTether();
  const { c } = useColors();
  const [addr, setAddr] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const slotProps = { accentColor: c.ink, inkColor: c.ink, slotColor: c.track, digitColor: c.ink, dangerColor: c.danger, slotSize: 48, gap: 8, radius: 14 };

  const join = (code: string) => {
    setBusy(true);
    setError(null);
    backend.join(code, addr.trim() || null).then(
      (device) => {
        const name = useTether.getState().devices.find((d) => d.id === device)?.name ?? device.slice(0, 12);
        toast(`Paired with ${name}`, "Your clipboard now syncs with it.");
        onClose();
      },
      (e) => {
        setBusy(false);
        setError(errorText(e));
        setAttempt((n) => n + 1); // clears the slots for another try
      },
    );
  };

  return (
    <div className="flex flex-col items-center gap-5">
      <CodeSlots key={attempt} length={6} autoFocus disabled={busy} status={error ? "error" : undefined} {...slotProps} onComplete={join} />
      <label className="flex w-full max-w-[340px] flex-col gap-1.5 text-[12px] text-subtle">
        Address, if it isn't found on your network (optional)
        <input
          value={addr}
          onChange={(e) => setAddr(e.target.value)}
          placeholder="192.168.1.20:5000"
          maxLength={64}
          spellCheck={false}
          disabled={busy}
          className="selectable h-9 rounded-xl border border-border bg-surface-2 px-3 font-mono text-[13px] text-foreground outline-none focus:border-border-hover"
        />
      </label>
      <p className={`min-h-[18px] text-center text-[12.5px] ${error ? "text-destructive" : "text-subtle"}`} role={error ? "alert" : undefined}>
        {busy ? "Looking for the device and checking the code…" : error ?? ""}
      </p>
    </div>
  );
}

function Body({ onClose }: { onClose: () => void }) {
  const live = useTether((s) => s.backend.live);
  const [mode, setMode] = useState<"show" | "enter">("show");
  return (
    <div className="flex flex-col gap-6">
      <p className="-mt-3 text-[13.5px] text-muted-foreground">
        {mode === "show"
          ? "Open Tether on the other device, choose Enter a code, and type this code. It works once and expires in two minutes."
          : "Type the six-digit code that the other device is showing."}
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
      <div className="flex min-h-[64px] justify-center">{mode === "show" ? <ShowCode onClose={onClose} /> : <EnterCode onClose={onClose} />}</div>
      {!live && <p className="text-center text-[11.5px] text-subtle">Preview: pairing works in the Tether desktop app.</p>}
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
