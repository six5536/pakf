// Runs the tests of a pack with `node --test`, in a project that syncs the packs, only
// while the project's copy of a file of the pack's code differs from the pack, because
// the pack repository tests each pack before any project syncs it. `PACK_TESTS` runs
// them at any time: `all` for every pack, or a list of pack ids that commas separate.
// A task that skips the tests says so, and why. A pack that pakf records no base for,
// as in a public mirror, runs its tests, because nothing then shows that its files are
// the ones that the pack repository tested.
//
// Usage: pack-tests.mts <pack id> <argument of node --test>...

import { spawnSync } from "node:child_process";
import process from "node:process";

type Env = Record<string, string | undefined>;

export interface PackReport {
  name: string;
  files: { path: string; state: string }[];
}

export type PackTests = { run: true; note?: string } | { run: false; skipped: string };

// The states of a file that differs from its base, whatever its acceptance, because an
// accepted edit is code that differs too. An accepted file that holds its base is
// `unchanged`, and a file that a higher pack shadows is listed apart, under `shadowed`.
const DIFFERS = new Set(["edited", "conflicted", "deleted"]);

// A file of a pack's code lies outside /knowledge/ and isn't Markdown, the rule by which
// the commit hook tells code from documents, because the tests read no document.
export function isCode(path: string): boolean {
  return !path.startsWith("/knowledge/") && !path.endsWith(".md");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// The entries of `pakf status --json`, each with the name of its pack and the path and
// the state of each file that differs from its base or has an acceptance.
export function parseReport(text: string): PackReport[] {
  const report: unknown = JSON.parse(text);
  if (!Array.isArray(report)) {
    throw new TypeError("the report isn't a list of packs");
  }
  return report.map((entry: unknown) => {
    if (!isRecord(entry) || typeof entry.name !== "string" || !Array.isArray(entry.files)) {
      throw new TypeError("an entry of the report has no name or no list of files");
    }
    const { name } = entry;
    const files = entry.files.map((file: unknown) => {
      if (!isRecord(file) || typeof file.path !== "string" || typeof file.state !== "string") {
        throw new TypeError(`a file of ${name} has no path or no state`);
      }
      return { path: file.path, state: file.state };
    });
    return { name, files };
  });
}

// The pack ids that PACK_TESTS names, or `all`, without blanks. An empty value names
// none, as an unset one does.
export function asked(env: Env): string[] {
  return (env.PACK_TESTS ?? "")
    .split(",")
    .map((id) => id.trim())
    .filter((id) => id !== "");
}

// Whether the tests of the pack `pack` run in the environment `env`, by the report of
// pakf. An id of PACK_TESTS that the report doesn't hold is refused, because a mistyped
// id would otherwise skip the tests without a word.
export function packTests(pack: string, report: PackReport[], env: Env): PackTests {
  const task =
    env.MISE_TASK_NAME === undefined || env.MISE_TASK_NAME === ""
      ? "pack-tests"
      : env.MISE_TASK_NAME;
  const ids = asked(env);
  const known = new Set(report.map((entry) => entry.name));
  const unknown = ids.filter((id) => id !== "all" && !known.has(id));
  if (unknown.length > 0) {
    throw new RangeError(
      `PACK_TESTS names ${unknown.join(", ")}, which pakf status doesn't report; it holds all, or pack ids that commas separate`,
    );
  }
  if (ids.includes("all") || ids.includes(pack)) {
    return { run: true };
  }
  const entry = report.find((candidate) => candidate.name === pack);
  if (entry === undefined) {
    return {
      run: true,
      note: `${task}: runs the tests of ${pack}, because pakf records no base for the pack.`,
    };
  }
  if (entry.files.some((file) => DIFFERS.has(file.state) && isCode(file.path))) {
    return { run: true };
  }
  return {
    run: false,
    skipped: `${task}: skipped the tests of ${pack}, because no file of its code differs from the pack; PACK_TESTS=${pack} runs them.`,
  };
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// The report of pakf, which runs through a shell only on Windows, where npm's shim of a
// tool is a .cmd file that Node.js runs only through a shell. There the command is one
// string, because Node.js deprecates arguments passed beside a shell.
function readReport(pack: string): PackReport[] {
  const options = { encoding: "utf8", maxBuffer: 2 ** 26 } as const;
  const status =
    process.platform === "win32"
      ? spawnSync("pakf status --json", { ...options, shell: true })
      : spawnSync("pakf", ["status", "--json"], options);
  const failure = `pakf failed to report the state of the files of ${pack}`;
  if (status.error !== undefined) {
    throw new Error(`${failure}: ${status.error.message}`);
  }
  if (status.status !== 0) {
    const output = `${status.stderr}${status.stdout}`.trim();
    const cause =
      output === "" ? `it exited with ${String(status.status ?? status.signal)}` : output;
    throw new Error(`${failure}: ${cause}`);
  }
  try {
    return parseReport(status.stdout);
  } catch (error) {
    throw new Error(`${failure}: ${messageOf(error)}`, { cause: error });
  }
}

function main(): number {
  const [pack = "", ...args] = process.argv.slice(2);
  if (pack === "" || args.length === 0) {
    console.error("pack-tests: usage: pack-tests.mts <pack id> <argument of node --test>...");
    return 2;
  }
  // A PACK_TESTS of only `all`, as a coverage task sets it, needs no report of pakf.
  const ids = asked(process.env);
  const all = ids.length > 0 && ids.every((id) => id === "all");
  let tests: PackTests;
  try {
    tests = all ? { run: true } : packTests(pack, readReport(pack), process.env);
  } catch (error) {
    console.error(`pack-tests: ${messageOf(error)}`);
    return 1;
  }
  if (!tests.run) {
    console.log(tests.skipped);
    return 0;
  }
  if (tests.note !== undefined) {
    console.log(tests.note);
  }
  const result = spawnSync(process.execPath, ["--test", ...args], { stdio: "inherit" });
  if (result.error !== undefined) {
    console.error(`pack-tests: node --test didn't start: ${result.error.message}`);
    return 1;
  }
  if (result.signal !== null) {
    console.error(`pack-tests: node --test stopped on ${result.signal}`);
  }
  return result.status ?? 1;
}

// exitCode, not exit, so that a message to a pipe is written in full first.
if (import.meta.main) {
  process.exitCode = main();
}
