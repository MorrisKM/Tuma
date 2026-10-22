import { useScrollRow } from "./ui";

interface Props<T extends string> {
  tabs: { id: T; label: string; count?: number }[];
  active: T;
  onChange: (id: T) => void;
  children?: React.ReactNode;
}

/** A row of tabs. `children` go on the right, for controls that belong to the active tab. */
export function Tabs<T extends string>({ tabs, active, onChange, children }: Props<T>) {
  const list = useScrollRow<HTMLDivElement>(".tab.active", [active]);
  return (
    <div className="tabs">
      <div className="tabs-list" role="tablist" ref={list}>
        {tabs.map((tab) => (
          <button
            key={tab.id}
            role="tab"
            aria-selected={tab.id === active}
            className={`tab${tab.id === active ? " active" : ""}`}
            onClick={() => onChange(tab.id)}
          >
            {tab.label}
            {tab.count ? <span className="count">{tab.count}</span> : null}
          </button>
        ))}
      </div>
      {children && <div className="tabs-extra">{children}</div>}
    </div>
  );
}
