import { useEffect, useRef } from "react";

interface ResizeHandleProps {
  axis: "horizontal" | "vertical";
  label: string;
  value: number;
  min: number;
  max: number;
  direction?: 1 | -1;
  onChange(value: number): void;
}

export function ResizeHandle({
  axis,
  label,
  value,
  min,
  max,
  direction = 1,
  onChange,
}: ResizeHandleProps) {
  const drag = useRef<{ start: number; value: number } | null>(null);

  useEffect(() => {
    const move = (event: PointerEvent) => {
      if (drag.current === null) return;
      const coordinate = axis === "horizontal" ? event.clientX : event.clientY;
      onChange(drag.current.value + (coordinate - drag.current.start) * direction);
    };
    const end = () => {
      drag.current = null;
      document.body.classList.remove("workbench-resizing");
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      document.body.classList.remove("workbench-resizing");
    };
  }, [axis, direction, onChange]);

  return (
    <div
      className={`workbench-resize workbench-resize--${axis}`}
      role="separator"
      aria-label={label}
      aria-orientation={axis === "horizontal" ? "vertical" : "horizontal"}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      tabIndex={0}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture?.(event.pointerId);
        drag.current = {
          start: axis === "horizontal" ? event.clientX : event.clientY,
          value,
        };
        document.body.classList.add("workbench-resizing");
      }}
      onKeyDown={(event) => {
        const decrement = axis === "horizontal" ? "ArrowLeft" : "ArrowUp";
        const increment = axis === "horizontal" ? "ArrowRight" : "ArrowDown";
        if (event.key !== decrement && event.key !== increment) return;
        event.preventDefault();
        onChange(value + (event.key === increment ? 10 : -10) * direction);
      }}
    />
  );
}
