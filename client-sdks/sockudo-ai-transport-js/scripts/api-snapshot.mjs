import { readFile } from "node:fs/promises";
import { normalize as normalizePath, posix, relative } from "node:path";

/** Package entry points, in the order they appear in the snapshot. */
export const ENTRIES = [
  ["core", "dist/index.d.ts"],
  ["react", "dist/react/index.d.ts"],
  ["vue", "dist/vue/index.d.ts"],
  ["svelte", "dist/svelte/index.d.ts"],
  ["vercel", "dist/vercel/index.d.ts"],
  ["vercel/react", "dist/vercel/react/index.d.ts"],
  ["vercel/vue", "dist/vercel/vue/index.d.ts"],
  ["vercel/svelte", "dist/vercel/svelte/index.d.ts"],
  ["providers", "dist/providers/index.d.ts"],
];

export const SNAPSHOT_PATH = "etc/api-snapshot.d.ts";

export const normalize = (content) => content.replaceAll("\r\n", "\n").trimEnd();

/**
 * Matches both re-export forms that hide declarations from the snapshot.
 *
 * `export *` hides the symbols entirely. A named `export { … } from` lists the
 * names but not their shapes, so on its own it would let a signature change
 * through — a generic gaining a parameter, or a method's argument type
 * changing — while the gate still passed. Following both pulls in the files
 * where the declarations actually live.
 */
const RE_EXPORT = /^export (?:\*|\{[\s\S]*?\}) from "(\.[^"]+)";$/gmu;

/**
 * Resolves a relative `export *` specifier to its declaration file.
 *
 * Declarations reference the emitted `.js` path, so the extension is swapped.
 */
function resolveDeclaration(fromFile, specifier) {
  const resolved = posix.normalize(posix.join(posix.dirname(fromFile), specifier));
  return resolved.replace(/\.js$/u, ".d.ts");
}

/**
 * Collects every declaration file reachable from an entry through a re-export.
 *
 * Entry declarations mostly re-export rather than declare, so a snapshot built
 * from entry files alone could not see most of this package's surface, nor any
 * signature. Following the chain is what makes the gate able to fail on a change
 * inside `core/transport` or `realtime`.
 */
async function collectReExports(file, seen) {
  const content = normalize(await readFile(file, "utf8"));
  const found = [];
  for (const match of content.matchAll(RE_EXPORT)) {
    const target = resolveDeclaration(file, match[1]);
    if (seen.has(target)) {
      continue;
    }
    seen.add(target);
    found.push(target);
    found.push(...(await collectReExports(target, seen)));
  }
  return found;
}

/**
 * Builds the public API snapshot.
 *
 * Two sections. Entry declarations first, in declaration order, unchanged from
 * how they are emitted. Then every module reachable through a re-export, each
 * included once and sorted by path so the output is deterministic regardless of
 * traversal order.
 */
export async function buildSnapshot() {
  const entrySections = await Promise.all(
    ENTRIES.map(async ([name, path]) => {
      const content = await readFile(path, "utf8");
      return `// ${name}\n${normalize(content)}`;
    }),
  );

  const seen = new Set(ENTRIES.map(([, path]) => normalizePath(path)));
  const reachable = new Set();
  for (const [, path] of ENTRIES) {
    for (const target of await collectReExports(path, seen)) {
      reachable.add(target);
    }
  }

  const starSections = await Promise.all(
    [...reachable].sort().map(async (path) => {
      const content = await readFile(path, "utf8");
      const label = relative("dist", path).replaceAll("\\", "/");
      return `// via re-export: ${label}\n${normalize(content)}`;
    }),
  );

  return [...entrySections, ...starSections].join("\n\n") + "\n";
}
