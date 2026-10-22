// Automatic updates: a check shortly after start (unless turned off), and installing with progress.

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useStore } from "./store";

export interface UpdateStatus {
  /** Whether this build can update itself: release builds only. */
  enabled: boolean;
  current: string;
}

export interface Available {
  version: string;
  notes: string | null;
  date: string | null;
}

export const updateStatus = () => invoke<UpdateStatus>("update_status");
export const checkForUpdate = () => invoke<Available | null>("update_check");
/** Installs the update the last check found and restarts the app. */
export const installUpdate = () => invoke<void>("update_install");

const KEY = "tuma.autoUpdateCheck";

export function autoCheckEnabled(): boolean {
  try {
    return localStorage.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

export function setAutoCheck(on: boolean) {
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    // Not remembered; it still applies now.
  }
}

/** Offers an available update in the notice bar. */
export function offer(update: Available) {
  useStore.getState().notify({
    tone: "info",
    text: `Tuma ${update.version} is available.`,
    details: update.notes ? [update.notes.split("\n")[0]] : undefined,
    action: { label: "Update and restart", run: () => void install() },
  });
}

export async function install() {
  const { notify } = useStore.getState();
  const stop = await listen<{ downloaded: number; total: number | null }>("update-progress", ({ payload }) => {
    const percent = payload.total ? ` ${Math.round((payload.downloaded / payload.total) * 100)}%` : "";
    notify({ tone: "info", text: `Downloading the update…${percent}` });
  });
  try {
    await installUpdate();
  } catch (e) {
    notify({ tone: "error", text: String(e) });
  } finally {
    stop();
  }
}

/** The check at start: quiet unless there is something to install. */
export async function checkAtStart() {
  if (!autoCheckEnabled()) return;
  const status = await updateStatus();
  if (!status.enabled) return;
  try {
    const update = await checkForUpdate();
    if (update) offer(update);
  } catch {
    // Offline, or GitHub is unreachable: try again next start.
  }
}
