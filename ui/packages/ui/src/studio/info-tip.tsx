import * as React from "react"
import { Info } from "lucide-react"

import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@workspace/ui/components/tooltip"

/**
 * An explanation kept out of the way behind a small info icon. It opens on
 * hover and on keyboard focus like any tooltip, and also on a tap, so it
 * works on touch screens; a tap elsewhere closes it.
 *
 * Safe inside a <label>: clicking the icon does not move the focus to the
 * labelled field.
 */
export function InfoTip({
  children,
  label = "What is this?",
}: {
  children: React.ReactNode
  label?: string
}) {
  const [open, setOpen] = React.useState(false)
  return (
    <TooltipProvider delayDuration={200} skipDelayDuration={300}>
      <Tooltip open={open} onOpenChange={setOpen}>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label={label}
            onClick={(e) => {
              e.preventDefault()
              setOpen(true)
            }}
            className="inline-flex size-4 shrink-0 items-center justify-center rounded-full text-subtle transition-colors duration-150 outline-none hover:text-foreground focus-visible:text-jade-hover"
          >
            <Info className="size-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent
          side="right"
          align="start"
          sideOffset={6}
          collisionPadding={12}
          className="block w-72 max-w-[calc(100vw-24px)] py-2.5 text-left text-[12px] leading-snug font-normal tracking-normal normal-case"
        >
          {children}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

/** The body of a parameter's info card. */
export function InfoCard({
  title,
  about,
  effects,
  tip,
}: {
  title: string
  about?: string
  effects?: [string, string][]
  tip?: string
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="font-semibold">{title}</div>
      {about && <p className="text-muted-foreground">{about}</p>}
      {effects && effects.length > 0 && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-1">
          {effects.map(([when, what]) => (
            <React.Fragment key={when}>
              <dt className="font-medium text-jade-hover">{when}</dt>
              <dd>{what}</dd>
            </React.Fragment>
          ))}
        </dl>
      )}
      {tip && (
        <p className="border-t border-line pt-2 text-muted-foreground">
          <span className="font-medium text-foreground">Tip: </span>
          {tip}
        </p>
      )}
    </div>
  )
}
