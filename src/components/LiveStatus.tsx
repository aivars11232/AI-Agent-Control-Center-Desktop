import type { ReactNode } from "react";

export function LiveStatus({
  children,
  className,
  kind = "status",
}: {
  children: ReactNode;
  className?: string;
  kind?: "status" | "error";
}) {
  if (!children) return null;

  return (
    <span
      className={className}
      role={kind === "error" ? "alert" : "status"}
      aria-live={kind === "error" ? "assertive" : "polite"}
      aria-atomic="true"
    >
      {children}
    </span>
  );
}
