// How the window is laid out: the sidebar's section and width, and how the request and response share the space.
// Remembered on this machine only.

import { create } from "zustand";

export type Section = "collections" | "environments" | "history";
export type Orientation = "stacked" | "side";

interface Layout {
  section: Section;
  sidebarOpen: boolean;
  /** In pixels. */
  sidebarWidth: number;
  orientation: Orientation;
  /** The request's share of the space it shares with the response, 0 to 1. */
  split: number;
}

const KEY = "tuma.layout";

const SIDEBAR_MIN = 220;
const SIDEBAR_MAX = 640;
/** Room the workbench keeps beside the rail and sidebar, however wide the sidebar is dragged. */
const WORKBENCH_MIN = 520;
const defaults: Layout = {
  section: "collections",
  sidebarOpen: true,
  sidebarWidth: 300,
  orientation: "stacked",
  split: 0.45,
};

function load(): Layout {
  try {
    return { ...defaults, ...(JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Layout>) };
  } catch {
    return defaults;
  }
}

export const useLayout = create<Layout>(load);

useLayout.subscribe((layout) => {
  try {
    localStorage.setItem(KEY, JSON.stringify(layout));
  } catch {
    // Not remembered; it still applies now.
  }
});

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export const layout = {
  /** Shows a section; choosing the one already shown hides the sidebar, as in editors. */
  show(section: Section) {
    const s = useLayout.getState();
    useLayout.setState(
      s.sidebarOpen && s.section === section ? { sidebarOpen: false } : { section, sidebarOpen: true },
    );
  },
  open: (section: Section) => useLayout.setState({ section, sidebarOpen: true }),
  toggleSidebar: () => useLayout.setState((s) => ({ sidebarOpen: !s.sidebarOpen })),
  setSidebarWidth: (width: number) =>
    useLayout.setState({
      sidebarWidth: clamp(Math.round(width), SIDEBAR_MIN, Math.min(SIDEBAR_MAX, window.innerWidth - WORKBENCH_MIN)),
    }),
  setSplit: (split: number) => useLayout.setState({ split: clamp(split, 0.12, 0.88) }),
  toggleOrientation: () =>
    useLayout.setState((s) => ({
      orientation: s.orientation === "stacked" ? "side" : "stacked",
      split: defaults.split,
    })),
  resetSplit: () => useLayout.setState({ split: defaults.split }),
  resetSidebar: () => useLayout.setState({ sidebarWidth: defaults.sidebarWidth }),
};
