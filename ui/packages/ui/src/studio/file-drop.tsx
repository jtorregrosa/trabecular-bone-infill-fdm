import * as React from "react"
import { cn } from "cn"

export const MESH_FILES = ".stl,.3mf"

/**
 * Files dropped anywhere on the page go to `onFile`. Returns whether a file is
 * being dragged over the page, and a function that opens the file picker.
 */
export function useFileDrop(onFile: (file: File) => void, accept = MESH_FILES) {
  const [dragging, setDragging] = React.useState(false)
  const input = React.useRef<HTMLInputElement | null>(null)
  const latest = React.useRef(onFile)
  React.useEffect(() => {
    latest.current = onFile
  }, [onFile])

  React.useEffect(() => {
    const over = (e: DragEvent) => {
      e.preventDefault()
      setDragging(true)
    }
    const leave = (e: DragEvent) => {
      e.preventDefault()
      setDragging(false)
    }
    const drop = (e: DragEvent) => {
      e.preventDefault()
      setDragging(false)
      const file = e.dataTransfer?.files[0]
      if (file) latest.current(file)
    }
    // Nothing on the page is meant to be dragged; files come from outside.
    const noDrag = (e: DragEvent) => e.preventDefault()
    window.addEventListener("dragenter", over)
    window.addEventListener("dragover", over)
    window.addEventListener("dragleave", leave)
    window.addEventListener("drop", drop)
    document.addEventListener("dragstart", noDrag)

    const picker = document.createElement("input")
    picker.type = "file"
    picker.accept = accept
    picker.addEventListener("change", () => {
      const file = picker.files?.[0]
      if (file) latest.current(file)
      picker.value = ""
    })
    input.current = picker
    return () => {
      window.removeEventListener("dragenter", over)
      window.removeEventListener("dragover", over)
      window.removeEventListener("dragleave", leave)
      window.removeEventListener("drop", drop)
      document.removeEventListener("dragstart", noDrag)
    }
  }, [accept])

  return { dragging, pick: () => input.current?.click() }
}

/** A dashed target that opens the file picker and lights up while dragging. */
export function DropZone({
  dragging,
  onClick,
  className,
  children,
}: {
  dragging: boolean
  onClick: () => void
  className?: string
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "w-full rounded-[10px] border border-dashed border-line-strong p-3.5 text-center transition-colors outline-none hover:border-subtle focus-visible:border-jade",
        dragging && "border-jade bg-jade-tint",
        className
      )}
    >
      {children}
    </button>
  )
}
