import * as React from "react"

import { ArrowUpRight, Leaf, X } from "lucide-react"

import { Button } from "@workspace/ui/components/button"

/**
 * Where the banner leads. Point it at the femur's model page once it is
 * published on MakerWorld; until then it opens the contest.
 */
const LINK = "https://makerworld.com/en/contests/170"
const LINK_TEXT = "See the contest"

/** Remembers that the banner was closed, in this browser only. */
const DISMISSED = "bone-infill:contest-banner-dismissed"

function wasDismissed() {
  try {
    return window.localStorage.getItem(DISMISSED) === "1"
  } catch {
    return false
  }
}

/** A slim strip above the app: bone-infill was made for a MakerWorld contest. */
export function ContestBanner() {
  const [open, setOpen] = React.useState(() => !wasDismissed())
  if (!open) return null

  const close = () => {
    setOpen(false)
    try {
      window.localStorage.setItem(DISMISSED, "1")
    } catch {
      // Private windows may refuse storage: the banner simply comes back.
    }
  }

  return (
    <div
      role="region"
      aria-label="About this project"
      className="flex min-h-9 items-center gap-2.5 border-b border-line bg-jade-tint py-1 pr-1.5 pl-4 text-[13px]"
    >
      <Leaf aria-hidden="true" className="size-3.5 shrink-0 text-jade" />
      <p className="min-w-0 flex-1 truncate">
        Made for MakerWorld&rsquo;s{" "}
        <strong className="font-semibold">Biomimicry Design Contest</strong>
        <span className="text-muted-foreground">
          {" "}
          · a femur with real bone inside, printed with no supports inside
        </span>
      </p>
      <a
        href={LINK}
        target="_blank"
        rel="noopener noreferrer"
        className="flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-medium text-jade hover:underline focus-visible:outline-2 focus-visible:outline-jade"
      >
        {LINK_TEXT}
        <ArrowUpRight aria-hidden="true" className="size-3.5" />
      </a>
      <Button
        variant="ghost"
        size="icon-xs"
        className="shrink-0 text-muted-foreground"
        aria-label="Close this banner"
        title="Close"
        onClick={close}
      >
        <X aria-hidden="true" />
      </Button>
    </div>
  )
}
