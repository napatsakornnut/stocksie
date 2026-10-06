"use client";

// SectionCard — the shared shell for the four feature tiles on /budget.
//
// Each of the four features (Approve / Reward / Track / Reimburse) is rendered
// inside this wrapper so the dashboard reads as a clean grid of equal-weight
// tiles. Mirrors the visual rhythm of the main `<Panel>` but with a tighter
// header (icon + title + one-line description) tuned for the denser grid layout.
//
// Pure presentational — no client-only imports beyond the `'use client'` directive
// (kept so it can be rendered alongside its client-only siblings without a
// boundary warning).

import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

export type SectionCardProps = {
  /** Tile title (e.g. "Approve"). Rendered as the visible heading. */
  title: string;
  /** Optional icon rendered before the title (typically an emoji). */
  icon?: ReactNode;
  /** One-line description shown under the title in muted text. */
  description?: ReactNode;
  /** Optional right-aligned slot for a small badge (e.g. "Admin only"). */
  badge?: ReactNode;
  /** Tile body. */
  children: ReactNode;
  /** Extra classes on the outer wrapper. */
  className?: string;
};

export function SectionCard({
  title,
  icon,
  description,
  badge,
  children,
  className,
}: SectionCardProps) {
  return (
    <section
      className={cn(
        "flex h-full flex-col overflow-hidden rounded-xl border border-stone-200 dark:border-slate-800",
        "bg-white dark:bg-slate-900/40 shadow-lg shadow-stone-900/5 dark:shadow-slate-950/20",
        className,
      )}
    >
      <header className="flex items-start justify-between gap-2 border-b border-stone-200 dark:border-slate-800 px-5 py-4">
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2">
            {icon && (
              <span className="text-base" aria-hidden="true">
                {icon}
              </span>
            )}
            <h2 className="text-sm font-semibold tracking-tight text-stone-800 dark:text-slate-100">
              {title}
            </h2>
          </div>
          {description && (
            <p className="max-w-prose text-xs leading-relaxed text-stone-500 dark:text-slate-400">
              {description}
            </p>
          )}
        </div>
        {badge}
      </header>
      <div className="flex flex-1 flex-col gap-3 p-5">{children}</div>
    </section>
  );
}
