// Light, dark, or following the system. Stored on this machine only.

export type Theme = "system" | "light" | "dark";

const KEY = "tuma.theme";

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
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Not remembered; it still applies now.
  }
}
