import type { KeyboardEvent, ReactNode } from "react";

export function KeyboardAction({
  children,
  className,
  label,
  onActivate,
}: {
  children: ReactNode;
  className?: string;
  label: string;
  onActivate: () => void;
}) {
  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onActivate();
  }

  return (
    <div
      className={className}
      role="button"
      tabIndex={0}
      aria-label={label}
      onClick={onActivate}
      onKeyDown={handleKeyDown}
    >
      {children}
    </div>
  );
}
