import { readFile, writeFile } from "node:fs/promises";

import { buildSnapshot, normalize, SNAPSHOT_PATH } from "./api-snapshot.mjs";

const write = process.argv.includes("--write");

const actualSnapshot = normalize(await buildSnapshot());

if (write) {
  await writeFile(SNAPSHOT_PATH, `${actualSnapshot}\n`);
  console.log(`Wrote ${SNAPSHOT_PATH}`);
  process.exit(0);
}

const expectedSnapshot = await readFile(SNAPSHOT_PATH, "utf8").then(normalize);

if (actualSnapshot !== expectedSnapshot) {
  console.error(
    "Public API snapshot mismatch. Run `node scripts/check-api-snapshot.mjs --write` if the API changed intentionally.",
  );
  const expectedLines = expectedSnapshot.split("\n");
  const actualLines = actualSnapshot.split("\n");
  // Dumping two ~3000-line declaration blobs buries the change, so print only
  // the differing lines.
  const removed = expectedLines.filter((line) => !actualLines.includes(line));
  const added = actualLines.filter((line) => !expectedLines.includes(line));
  for (const line of removed.slice(0, 40)) {
    console.error(`- ${line}`);
  }
  for (const line of added.slice(0, 40)) {
    console.error(`+ ${line}`);
  }
  const shown = Math.min(removed.length, 40) + Math.min(added.length, 40);
  if (removed.length + added.length > shown) {
    console.error(`… ${removed.length + added.length - shown} more changed lines`);
  }
  process.exitCode = 1;
}
