import { useStore } from "./store";
import type { TreeNode } from "./types";

/** Where on a row the pointer is: its top edge, its bottom edge, or (for folders and collections) its middle. */
export type Zone = "before" | "after" | "into";

/** Where a dropped item goes: into `parent` (null for the top level), before the item named `before` (null: last). */
export interface Drop {
  parent: string | null;
  before: string | null;
}

/** The node at `path` and the list it is in, with its parent's path (null at the top level). */
function locate(tree: TreeNode[], path: string, parent: string | null = null): { siblings: TreeNode[]; parent: string | null } | null {
  for (const node of tree) {
    if (node.path === path) return { siblings: tree, parent };
    const found = node.children && locate(node.children, path, node.path);
    if (found) return found;
  }
  return null;
}

/** Where `dragged` goes when dropped on `zone` of the row of `target`, or null if it can't go there. */
export function dropFor(tree: TreeNode[], dragged: TreeNode, target: TreeNode, zone: Zone): Drop | null {
  if (target.path === dragged.path || target.path.startsWith(`${dragged.path}/`)) return null;
  // Requests live in collections and folders, so the edges of a collection row mean "inside it" for them.
  if (zone === "into" || (dragged.kind === "request" && target.kind === "collection")) {
    return target.kind === "request" ? null : { parent: target.path, before: null };
  }
  const place = locate(tree, target.path);
  if (!place) return null;
  if (zone === "before") return { parent: place.parent, before: target.name };
  const next = place.siblings.slice(place.siblings.indexOf(target) + 1).find((n) => n.path !== dragged.path);
  return { parent: place.parent, before: next?.name ?? null };
}

/** The zone of a row the pointer is over: thirds for folders and collections, halves for requests. */
function zoneAt(row: HTMLElement, y: number, kind: TreeNode["kind"]): Zone {
  const r = row.getBoundingClientRect();
  const at = (y - r.top) / r.height;
  if (kind === "request") return at < 0.5 ? "before" : "after";
  return at < 0.3 ? "before" : at > 0.7 ? "after" : "into";
}

const MARKS = ["drop-before", "drop-after", "drop-into"];

/** The node at `path` anywhere in the tree. */
function find(tree: TreeNode[], path: string): TreeNode | undefined {
  for (const node of tree) {
    if (node.path === path) return node;
    const found = node.children && find(node.children, path);
    if (found) return found;
  }
  return undefined;
}

/**
 * Lets a tree row be dragged to another place, from a press on it. Uses pointer events rather than HTML drag and drop,
 * which the Windows webview keeps for files dropped from outside.
 */
export function startDrag(event: React.PointerEvent<HTMLElement>, node: TreeNode) {
  if (event.button !== 0 || (event.target as HTMLElement).closest("button, input")) return;
  const startX = event.clientX;
  const startY = event.clientY;
  let ghost: HTMLDivElement | null = null;
  let marked: HTMLElement | null = null;
  let drop: Drop | null = null;
  let hover: { path: string; timer: number } | null = null;

  const unmark = () => {
    marked?.classList.remove(...MARKS);
    marked = null;
  };
  const stop = () => {
    removeEventListener("pointermove", move);
    removeEventListener("pointerup", up);
    removeEventListener("keydown", key, true);
    ghost?.remove();
    unmark();
    if (hover) clearTimeout(hover.timer);
    document.body.classList.remove("tree-dragging");
  };

  const move = (e: PointerEvent) => {
    if (!ghost) {
      if (Math.hypot(e.clientX - startX, e.clientY - startY) < 5) return;
      ghost = document.createElement("div");
      ghost.className = "drag-ghost";
      ghost.textContent = node.name;
      document.body.append(ghost);
      document.body.classList.add("tree-dragging");
    }
    ghost.style.transform = `translate(${e.clientX + 12}px, ${e.clientY + 8}px)`;
    unmark();
    drop = null;
    const row = (document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null)?.closest<HTMLElement>(".tree-row[data-path]");
    const target = row ? find(useStore.getState().tree, row.dataset.path!) : undefined;
    if (!row || !target) return;
    const zone = zoneAt(row, e.clientY, target.kind);
    drop = dropFor(useStore.getState().tree, node, target, zone);
    if (!drop) return;
    const into = drop.parent === target.path && drop.before === null;
    row.classList.add(into ? "drop-into" : zone === "before" ? "drop-before" : "drop-after");
    marked = row;
    // Resting on a closed folder opens it, to drop deeper inside.
    if (into && hover?.path !== target.path) {
      if (hover) clearTimeout(hover.timer);
      hover = { path: target.path, timer: window.setTimeout(() => useStore.getState().toggle(target.path, true), 650) };
    } else if (!into && hover) {
      clearTimeout(hover.timer);
      hover = null;
    }
  };

  const up = () => {
    const dragged = !!ghost;
    const target = drop;
    stop();
    if (!dragged) return;
    // The click that ends a drag (if the browser sends one) shouldn't open or toggle the row under it.
    const swallow = (e: MouseEvent) => e.stopPropagation();
    addEventListener("click", swallow, { capture: true, once: true });
    setTimeout(() => removeEventListener("click", swallow, { capture: true }));
    if (target) void useStore.getState().move(node.path, target.parent, target.before);
  };

  const key = (e: KeyboardEvent) => {
    if (e.key === "Escape" && ghost) {
      e.stopPropagation();
      drop = null;
      ghost.remove();
      ghost = null;
      stop();
    }
  };

  addEventListener("pointermove", move);
  addEventListener("pointerup", up);
  addEventListener("keydown", key, true);
}
