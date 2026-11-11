import * as React from "react"
import { cn } from "cn"

/** Section heading inside a side panel. */
export function PanelHeading({
  className,
  ...props
}: React.ComponentProps<"h2">) {
  return (
    <h2
      className={cn(
        "mt-[18px] mb-2 text-[11px] font-semibold tracking-[0.08em] text-subtle uppercase",
        className
      )}
      {...props}
    />
  )
}

/** Explanatory text under a control. */
export function Hint({ className, ...props }: React.ComponentProps<"p">) {
  return (
    <p
      className={cn("text-[11.5px] leading-snug text-subtle", className)}
      {...props}
    />
  )
}

/**
 * A side panel. Text is not selectable: a stray selection makes the next drag
 * on a slider drag the selection instead.
 */
export function Panel({ className, ...props }: React.ComponentProps<"aside">) {
  return (
    <aside
      className={cn("flex min-h-0 flex-col bg-surface select-none", className)}
      {...props}
    />
  )
}

/** The scrolling body of a panel. */
export function PanelBody({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-4 pt-3.5 pb-6",
        className
      )}
      {...props}
    />
  )
}

/** Actions kept in view at the foot of a panel. */
export function PanelFooter({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "border-t border-line bg-surface px-4 pt-2.5 pb-3",
        className
      )}
      {...props}
    />
  )
}

export function Spinner({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block size-[11px] shrink-0 animate-spin rounded-full border-2 border-line-strong border-t-jade",
        className
      )}
    />
  )
}
