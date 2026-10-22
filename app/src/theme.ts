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
