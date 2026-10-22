// The JSON path subset Tuma's checks use (crates/core/src/expect.rs `select`): $, .name, ['name'], [0], [-1], [*], .*

type Step = { key: string } | { index: number } | { all: true };

function parse(path: string): Step[] {
  const rest = path.trim();
  if (!rest.startsWith("$")) throw new Error("A path starts with $, such as $.items[0].id");
  const steps: Step[] = [];
  let i = 1;
  while (i < rest.length) {
    const c = rest[i];
    if (c === ".") {
      let j = i + 1;
      while (j < rest.length && rest[j] !== "." && rest[j] !== "[") j++;
      const name = rest.slice(i + 1, j);
      if (!name) throw new Error("Empty name after .");
      steps.push(name === "*" ? { all: true } : { key: name });
      i = j;
    } else if (c === "[") {
      const end = rest.indexOf("]", i);
      if (end < 0) throw new Error("Unclosed [");
      const inner = rest.slice(i + 1, end).trim();
      const quoted = inner.match(/^(['"])(.*)\1$/);
      if (inner === "*") steps.push({ all: true });
      else if (quoted) steps.push({ key: quoted[2] });
      else if (/^-?\d+$/.test(inner)) steps.push({ index: Number(inner) });
      else throw new Error(`Bad index [${inner}]`);
      i = end + 1;
    } else {
      throw new Error(`Unexpected "${c}"`);
    }
  }
  return steps;
}

/** The values a path selects. Throws on a malformed path. */
export function select(root: unknown, path: string): unknown[] {
  let current: unknown[] = [root];
  for (const step of parse(path)) {
    current = current.flatMap((value) => {
      if ("key" in step) {
        return value && typeof value === "object" && !Array.isArray(value) && step.key in value
          ? [(value as Record<string, unknown>)[step.key]]
          : [];
      }
      if ("index" in step) {
        if (!Array.isArray(value)) return [];
        const i = step.index < 0 ? value.length + step.index : step.index;
        return i >= 0 && i < value.length ? [value[i]] : [];
      }
      if (Array.isArray(value)) return value;
      return value && typeof value === "object" ? Object.values(value) : [];
    });
  }
  return current;
}
