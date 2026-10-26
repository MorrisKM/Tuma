// Light, dark, or following the system. Stored on this machine only.

import { useSyncExternalStore } from "react";

export type Theme = "system" | "light" | "dark";

const KEY = "tuma.theme";
const listeners = new Set<() => void>();
let current: Theme = loadTheme();

export function loadTheme(): Theme {
  try {
    const value = localStorage.getItem(KEY);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    return "system";
  }
}

/** Sets data-theme on <html>, which App.css reads; "system" removes it so the media query decides. */
export function applyTheme(theme: Theme) {
  if (theme === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", theme);
  current = theme;
  listeners.forEach((f) => f());
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Not remembered; it still applies now.
  }
}

/** The chosen theme, re-rendering when it changes. */
export const useTheme = () =>
  useSyncExternalStore(
    (f) => {
      listeners.add(f);
      return () => listeners.delete(f);
    },
    () => current,
  );

// Colors: a palette tints the surfaces; the accent colors outlines, focus rings, buttons and selection. Also stored
// on this machine only.

export type Palette = "neutral" | "harbor" | "grove";
export type Accent =
  | "auto"
  | "tuma"
  | "molten-lava"
  | "flag-red"
  | "deep-space-blue"
  | "steel-blue"
  | "olive-leaf"
  | "black-forest"
  | "light-caramel"
  | "copper";

export const PALETTES: { id: Palette; label: string; description: string; swatches: string[]; accent: Accent }[] = [
  { id: "neutral", label: "Neutral", description: "Quiet grays", swatches: ["#ffffff", "#efefed", "#737373", "#121212"], accent: "tuma" },
  {
    id: "harbor",
    label: "Harbor",
    description: "Papaya Whip by day, Deep Space Blue by night",
    swatches: ["#780000", "#c1121f", "#fdf0d5", "#003049", "#669bbc"],
    accent: "flag-red",
  },
  {
    id: "grove",
    label: "Grove",
    description: "Cornsilk by day, Black Forest by night",
    swatches: ["#606c38", "#283618", "#fefae0", "#dda15e", "#bc6c25"],
    accent: "copper",
  },
];

/** Accent colors as they look in the light theme; the dark theme uses lighter versions (App.css). */
export const ACCENTS: { id: Exclude<Accent, "auto">; label: string; color: string }[] = [
  { id: "tuma", label: "Tuma Green", color: "#0c9a6e" },
  { id: "molten-lava", label: "Molten Lava", color: "#780000" },
  { id: "flag-red", label: "Flag Red", color: "#c1121f" },
  { id: "deep-space-blue", label: "Deep Space Blue", color: "#003049" },
  { id: "steel-blue", label: "Steel Blue", color: "#669bbc" },
  { id: "olive-leaf", label: "Olive Leaf", color: "#606c38" },
  { id: "black-forest", label: "Black Forest", color: "#283618" },
  { id: "light-caramel", label: "Light Caramel", color: "#dda15e" },
  { id: "copper", label: "Copper", color: "#bc6c25" },
];

export interface Colors {
  palette: Palette;
  /** "auto" takes the palette's accent. */
  accent: Accent;
}

const COLORS_KEY = "tuma.colors";
let colors: Colors = loadColors();

export function loadColors(): Colors {
  try {
    const value = JSON.parse(localStorage.getItem(COLORS_KEY) ?? "{}") as Partial<Colors>;
    return {
      palette: PALETTES.some((p) => p.id === value.palette) ? value.palette! : "neutral",
      accent: value.accent === "auto" || ACCENTS.some((a) => a.id === value.accent) ? value.accent! : "auto",
    };
  } catch {
    return { palette: "neutral", accent: "auto" };
  }
}

/** The accent shown: the chosen one, or the palette's. */
export const accentOf = (c: Colors): Exclude<Accent, "auto"> =>
  c.accent === "auto" ? (PALETTES.find((p) => p.id === c.palette)!.accent as Exclude<Accent, "auto">) : c.accent;

/** Sets data-palette and data-accent on <html>, which App.css reads; the defaults remove them. */
export function applyColors(next: Colors) {
  const root = document.documentElement;
  const accent = accentOf(next);
  if (next.palette === "neutral") root.removeAttribute("data-palette");
  else root.setAttribute("data-palette", next.palette);
  if (accent === "tuma") root.removeAttribute("data-accent");
  else root.setAttribute("data-accent", accent);
  colors = next;
  listeners.forEach((f) => f());
  try {
    localStorage.setItem(COLORS_KEY, JSON.stringify(next));
  } catch {
    // Not remembered; it still applies now.
  }
}

/** The chosen colors, re-rendering when they change. */
export const useColors = () =>
  useSyncExternalStore(
    (f) => {
      listeners.add(f);
      return () => listeners.delete(f);
    },
    () => colors,
  );
