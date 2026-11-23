import { type ButtonHTMLAttributes, type CSSProperties, type ReactElement, type ReactNode } from "react";
import { cn } from "@/lib/cn";
import { useTether } from "@/data/store";
import type { Connection, Platform } from "@/data/types";
import SpotlightCard from "./rb/SpotlightCard";
import SquishSwitch from "./rb/SquishSwitch";
import RubberSegment from "./rb/RubberSegment";
import WarmTooltip, { WarmTooltipGroup } from "./rb/WarmTooltip";
import { NumberTicker } from "./magic/number-ticker";
import { Icon, connectionGlyph, icons, platformGlyph } from "./Icon";

export { WarmTooltipGroup };

/** Concrete colours for libraries that need hex (shaders, contrast maths). Mirrors index.css. */
export const palette = {
  dark: { accent: "#30d158", ink: "#f2f2f4", surface: "#18181b", raised: "#2c2c31", track: "#2a2a2f", muted: "#a0a0a8", positive: "#30d158", danger: "#ff6961", sheen: "#ffffff" },
  light: { accent: "#248a3d", ink: "#1d1d1f", surface: "#ffffff", raised: "#ffffff", track: "#e5e5ea", muted: "#6e6e73", positive: "#248a3d", danger: "#d70015", sheen: "#000000" },
} as const;

export function useColors() {
  const theme = useTether((s) => s.theme);
  return { theme, c: palette[theme] };
}

/* ---------- Surfaces ---------- */

/** A card that catches a faint neutral sheen under the cursor. */
export function Card({ children, className, pad = 20, radius = 22, style, ...rest }: { children: ReactNode; className?: string; pad?: number; radius?: number; style?: CSSProperties } & React.HTMLAttributes<HTMLDivElement>) {
  const { theme, c } = useColors();
  return (
    <SpotlightCard
      theme={theme}
      spotlightColor={c.sheen}
      intensity={theme === "dark" ? 0.05 : 0.12}
      spotlightSize={320}
      borderGlow={0.3}
      flare={false}
      className={className}
      style={
        {
          padding: pad,
          borderRadius: radius,
          "--spotlight-card-surface": "var(--surface)",
          "--spotlight-card-border": "var(--border)",
          "--spotlight-card-shadow": "var(--shadow-lift)",
          ...style,
        } as CSSProperties
      }
      {...rest}
    >
      {children}
    </SpotlightCard>
  );
}

/** A quiet panel: hairline, no pointer effects. Use for dense lists. */
export function Panel({ children, className, ...rest }: { children: ReactNode; className?: string } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("rounded-[20px] border border-border bg-surface", className)} {...rest}>
      {children}
    </div>
  );
}

/* ---------- Buttons ---------- */

type Variant = "solid" | "soft" | "outline" | "ghost" | "danger";

const variants: Record<Variant, string> = {
  solid: "bg-foreground text-background hover:opacity-85",
  soft: "bg-muted text-foreground hover:bg-foreground/10",
  outline: "border border-border text-foreground hover:border-border-hover hover:bg-muted",
  ghost: "text-muted-foreground hover:bg-muted hover:text-foreground",
  danger: "text-destructive hover:bg-destructive/10",
};

interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  variant?: Variant;
  size?: "sm" | "md";
  icon?: (typeof icons)[keyof typeof icons];
  children?: ReactNode;
}

export function Button({ variant = "soft", size = "md", icon, className, children, ...props }: ButtonProps) {
  return (
    <button
      {...props}
      className={cn(
        "inline-flex shrink-0 items-center justify-center gap-2 rounded-full font-medium whitespace-nowrap",
        "transition-[background-color,color,border-color,filter,transform] duration-200 ease-out active:scale-[0.97]",
        "disabled:pointer-events-none disabled:opacity-40",
        size === "sm" ? "h-8 px-3.5 text-[13px]" : "h-10 px-5 text-sm",
        variants[variant],
        className,
      )}
    >
      {icon && <Icon icon={icon} size={size === "sm" ? 15 : 17} />}
      {children}
    </button>
  );
}

/** The one primary action on a page: an inverted pill. */
export function PrimaryAction({ children, icon, onClick, disabled, type = "button" }: { children: ReactNode; icon?: (typeof icons)[keyof typeof icons]; onClick?: () => void; disabled?: boolean; type?: "button" | "submit" }) {
  return (
    <Button variant="solid" icon={icon} onClick={onClick} disabled={disabled} type={type}>
      {children}
    </Button>
  );
}

export function IconButton({ icon, label, onClick, tone = "ghost", shortcut, className, disabled }: { icon: (typeof icons)[keyof typeof icons]; label: string; onClick?: () => void; tone?: "ghost" | "danger"; shortcut?: string; className?: string; disabled?: boolean }) {
  return (
    <WarmTooltip content={label} shortcut={shortcut} side="bottom" size="sm">
      <button
        aria-label={label}
        onClick={onClick}
        disabled={disabled}
        className={cn(
          "inline-flex size-9 items-center justify-center rounded-full transition-[background-color,color,transform] duration-200 active:scale-[0.92] disabled:opacity-40",
          tone === "danger" ? "text-muted-foreground hover:bg-destructive/10 hover:text-destructive" : "text-muted-foreground hover:bg-muted hover:text-foreground",
          className,
        )}
      >
        <Icon icon={icon} size={17} />
      </button>
    </WarmTooltip>
  );
}

/* ---------- Controls ---------- */

export function Switch({ checked, onChange, label, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  const { c } = useColors();
  return (
    <SquishSwitch checked={checked} onChange={onChange} ariaLabel={label} disabled={disabled} width={46} height={28} radius={14} trackColor={c.track} trackOnColor={c.positive} thumbColor="#ffffff" thumbOnColor="#ffffff" />
  );
}

export function Segmented<T extends string>({ value, onChange, options, label }: { value: T; onChange: (v: T) => void; options: { value: T; label: string }[]; label: string }) {
  const { c } = useColors();
  return (
    <RubberSegment
      items={options}
      value={value}
      onChange={(v) => onChange(v as T)}
      size="sm"
      trackColor={c.track}
      thumbColor={c.raised}
      textColor={c.muted}
      activeTextColor={c.ink}
      aria-label={label}
    />
  );
}

/** A large figure that springs to each new value. */
export function Figure({ value, decimals = 0, size = 44 }: { value: number; decimals?: number; size?: number }) {
  return <NumberTicker value={value} decimalPlaces={decimals} className="display text-foreground dark:text-foreground" style={{ fontSize: size, lineHeight: 1 }} />;
}

/* ---------- Small parts ---------- */

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="inline-flex h-5 items-center rounded-md border border-border px-1.5 font-mono text-[10.5px] text-subtle">{children}</kbd>;
}

export function Chip({ children, className, icon }: { children: ReactNode; className?: string; icon?: ReactElement }) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 rounded-full bg-muted px-2.5 py-[3px] text-xs text-muted-foreground", className)}>
      {icon}
      {children}
    </span>
  );
}

export function StatusDot({ connection, className }: { connection: Connection; className?: string }) {
  const online = connection !== "offline";
  return (
    <span className={cn("relative inline-flex size-2", className)} aria-hidden>
      {online && <span className="absolute inset-0 animate-ping rounded-full bg-positive opacity-40" style={{ animationDuration: "2.6s" }} />}
      <span className={cn("relative size-2 rounded-full", online ? "bg-positive" : "bg-subtle")} />
    </span>
  );
}

export const connectionLabel: Record<Connection, string> = { lan: "Local network", direct: "Direct", relay: "Relay", offline: "Offline" };

export function ConnectionBadge({ connection }: { connection: Connection }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
      <StatusDot connection={connection} />
      <Icon icon={connectionGlyph[connection]} size={13} />
      {connectionLabel[connection]}
    </span>
  );
}

export function DeviceGlyph({ platform, size = 40, online = true }: { platform: Platform; size?: number; online?: boolean }) {
  return (
    <span
      className={cn("inline-flex shrink-0 items-center justify-center rounded-full border border-border bg-surface-2", online ? "text-foreground" : "text-subtle")}
      style={{ width: size, height: size }}
    >
      <Icon icon={platformGlyph[platform]} size={Math.round(size * 0.46)} />
    </span>
  );
}

export function SectionLabel({ children, action }: { children: ReactNode; action?: ReactNode }) {
  return (
    <div className="mb-3 flex items-center justify-between px-1">
      <h2 className="text-[13px] font-semibold text-foreground">{children}</h2>
      {action}
    </div>
  );
}

export function EmptyState({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-1.5 p-10 text-center">
      <p className="text-lg font-semibold tracking-tight text-foreground">{title}</p>
      {hint && <p className="max-w-xs text-[13px] text-muted-foreground">{hint}</p>}
    </div>
  );
}

/** Page header: small eyebrow, bold title, quiet subtitle, actions on the right. */
export function PageHeader({ eyebrow, title, subtitle, actions }: { eyebrow?: string; title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-4 pb-8">
      <div className="min-w-0">
        {eyebrow && <p className="mb-2 text-[13px] font-medium text-muted-foreground">{eyebrow}</p>}
        <h1 className="display text-[clamp(2rem,3.2vw,2.6rem)] text-foreground">{title}</h1>
        {subtitle && <p className="mt-2.5 max-w-xl text-[15px] text-muted-foreground">{subtitle}</p>}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </header>
  );
}
