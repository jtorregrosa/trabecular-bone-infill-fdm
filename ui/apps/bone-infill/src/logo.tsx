import { cn } from "@workspace/ui/lib/utils"

// The logo from the brand pack (docs/brand), inlined at build time. It takes
// the colour of the text around it; in the app that is bone.
import logo from "@brand/svg/logo-currentcolor.svg?raw"

/** The bone-infill logo, as tall as its box (it is about 1 : 2.8). */
export function Logo({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block shrink-0 text-bone [&>svg]:h-full [&>svg]:w-auto",
        className
      )}
      dangerouslySetInnerHTML={{ __html: logo }}
    />
  )
}
