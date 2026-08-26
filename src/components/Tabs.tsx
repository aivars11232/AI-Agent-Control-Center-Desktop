import { useRef, type KeyboardEvent } from "react";

export type TabDefinition<T extends string> = {
  value: T;
  label: string;
};

function idPart(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

export function tabId(prefix: string, value: string): string {
  return `${prefix}-tab-${idPart(value)}`;
}

export function tabPanelId(prefix: string, value: string): string {
  return `${prefix}-panel-${idPart(value)}`;
}

export function Tabs<T extends string>({
  className = "workspace-tabs",
  idPrefix,
  label,
  onChange,
  tabs,
  value,
}: {
  className?: string;
  idPrefix: string;
  label: string;
  onChange: (value: T) => void;
  tabs: readonly TabDefinition<T>[];
  value: T;
}) {
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);

  function moveFocus(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight") {
      nextIndex = (index + 1) % tabs.length;
    } else if (event.key === "ArrowLeft") {
      nextIndex = (index - 1 + tabs.length) % tabs.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = tabs.length - 1;
    }
    if (nextIndex === null) return;

    event.preventDefault();
    const nextTab = tabs[nextIndex];
    onChange(nextTab.value);
    buttonRefs.current[nextIndex]?.focus();
  }

  return (
    <div className={className} role="tablist" aria-label={label}>
      {tabs.map((tab, index) => {
        const selected = tab.value === value;
        return (
          <button
            key={tab.value}
            ref={(element) => {
              buttonRefs.current[index] = element;
            }}
            id={tabId(idPrefix, tab.value)}
            type="button"
            role="tab"
            aria-controls={tabPanelId(idPrefix, tab.value)}
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            className={selected ? "primary-button" : "secondary-button"}
            onClick={() => onChange(tab.value)}
            onKeyDown={(event) => moveFocus(event, index)}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
