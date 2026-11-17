// Times one insight step in V8, the engine Workers run, on the Worker's own
// wasm target. No dependency: Node's WebAssembly and a child process per
// measurement, so each first call pays a fresh module compile and every
// pattern's compile, as a cold isolate would.
//
// Calibration cases are timed the same way: a no-op call (the fixed cost of
// a first call), the insight tick with nothing to do (its own path, no
// work), and two paths that already run in production: a share card render
// from a real cover (`cover::card`), and the number check a recap build runs
// on its draft (`numbers::check`), on the published recap of 2026-11-06 and
// its facts.
//
//   node run.mjs WASM SNAPSHOT_DIR            the steps and the calibration
//   node run.mjs WASM KIND INPUT --one        one case (used by the above)
import { readFileSync, readdirSync, writeFileSync, mkdtempSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const WARM_RUNS = 20;
const here = dirname(fileURLToPath(import.meta.url));

function instantiate(wasmPath) {
  const module = new WebAssembly.Module(readFileSync(wasmPath));
  // wasm-bindgen's imports (pulled in through chrono's default features)
  // are never called here: each is a stub that fails if it is.
  const imports = {};
  for (const i of WebAssembly.Module.imports(module)) {
    (imports[i.module] ??= {})[i.name] = () => {
      throw new Error(`unexpected call to ${i.module}.${i.name}`);
    };
  }
  return new WebAssembly.Instance(module, imports).exports;
}

// One case: `kind` is step (an insight step snapshot), card, draft or noop.
function measureOne(wasmPath, kind, input) {
  const x = instantiate(wasmPath);
  if (input) {
    const bytes = readFileSync(input);
    const ptr = x.alloc(bytes.length);
    new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
    const parsed = x.load(ptr, bytes.length);
    if (kind === "step" && parsed !== 1) throw new Error(`bad snapshot ${input}`);
  }
  const before = { step: () => x.prepare(), draft: () => x.prepare_draft(), card: () => {}, noop: () => {} }[kind];
  const call = { step: () => x.tick(), draft: () => x.draft(), card: () => x.card(), noop: () => x.noop() }[kind];
  before();
  const t1 = performance.now();
  const result = call();
  const first = performance.now() - t1;
  if (kind === "step" && result >= 1_000_000) throw new Error(`the tick failed on ${input}`);
  if (kind === "card" && result === 0) throw new Error("the card did not render");
  const warm = [];
  for (let i = 0; i < WARM_RUNS; i++) {
    before();
    const t = performance.now();
    call();
    warm.push(performance.now() - t);
  }
  warm.sort((a, b) => a - b);
  return { first, warm: warm[Math.floor(warm.length / 2)], result };
}

const args = process.argv.slice(2);
if (args[3] === "--one") {
  const [wasmPath, kind, input] = args;
  console.log(JSON.stringify(measureOne(wasmPath, kind, input === "-" ? null : input)));
} else {
  const [wasmPath, snapshots] = args;
  const one = (kind, input) =>
    JSON.parse(execFileSync(process.execPath, [process.argv[1], wasmPath, kind, input ?? "-", "--one"]).toString());
  const rows = {};
  const add = (row, post, r) => {
    (rows[row] ??= []).push({ post, ...r });
    console.log(`${post} ${row}: first ${r.first.toFixed(2)} ms, warm ${r.warm.toFixed(3)} ms`);
  };
  for (const f of readdirSync(snapshots).filter((f) => f.endsWith(".json")).sort()) {
    const [post, step] = f.replace(/\.json$/, "").split(".");
    add(step.replace(/^\d-/, ""), post, one("step", join(snapshots, f)));
  }
  // Calibration: 13 fresh processes each, as many as the stored posts.
  const tmp = mkdtempSync(join(tmpdir(), "insight-cpu-"));
  const draft = join(tmp, "draft.json");
  writeFileSync(
    draft,
    JSON.stringify({
      facts: JSON.parse(readFileSync(join(here, "../../crates/core/src/insight/testdata/shadow/recap_2026-11-06.json"))),
      body: readFileSync(join(here, "calibration/recap_2026-11-06.en.md"), "utf8"),
    }),
  );
  const cover = join(here, "calibration/recap_2026-11-06.cover.svg");
  // The insight step with nothing to do: its own code path, no work.
  const idle = join(tmp, "idle.json");
  writeFileSync(
    idle,
    JSON.stringify({ posts: [], objects: {}, calendar: [], checks: [], kv: {}, writer: null, judge: null, now: "2026-11-13T12:00:00+00:00" }),
  );
  // An idle sweep on a normal afternoon: six bundles of two days, each with
  // a done record (the step's own pinned record, about 30 KB), nothing to do.
  const done = readFileSync(join(here, "../../crates/core/src/insight/testdata/shadow/record.json"), "utf8");
  const posts = [];
  const objects = {};
  for (const d of ["2026-11-12", "2026-11-13"]) {
    posts.push(["premarket", d, `premarket-${d}`, `premarket/${d}-update.json`], ["recap", d, `recap-${d}`, `recap/${d}.json`]);
    for (const k of [`premarket/${d}.json`, `premarket/${d}-update.json`, `recap/${d}.json`]) {
      objects[`insights/${k}`] = done;
    }
  }
  const idle6 = join(tmp, "idle6.json");
  writeFileSync(
    idle6,
    JSON.stringify({ posts, objects, calendar: [], checks: [], kv: {}, writer: null, judge: null, now: "2026-11-13T22:00:00+00:00" }),
  );
  for (let i = 0; i < 13; i++) {
    add("calibration: no-op call", `run ${i + 1}`, one("noop", null));
    add("calibration: insight tick, nothing to do", `run ${i + 1}`, one("step", idle));
    add("idle sweep: six done records", `run ${i + 1}`, one("step", idle6));
    add("calibration: share card", `run ${i + 1}`, one("card", cover));
    add("calibration: draft number check", `run ${i + 1}`, one("draft", draft));
  }
  const stat = (v) => {
    const s = [...v].sort((a, b) => a - b);
    return { worst: s[s.length - 1], median: s[Math.floor(s.length / 2)] };
  };
  console.log("\n| Case | Runs | First call, worst | First call, median | Warm, worst | Warm, median |");
  console.log("|---|---|---|---|---|---|");
  for (const [row, rs] of Object.entries(rows)) {
    const f = stat(rs.map((r) => r.first));
    const w = stat(rs.map((r) => r.warm));
    console.log(
      `| ${row} | ${rs.length} | ${f.worst.toFixed(2)} ms | ${f.median.toFixed(2)} ms | ${w.worst.toFixed(3)} ms | ${w.median.toFixed(3)} ms |`,
    );
  }
  console.log(`\nNode ${process.version} (V8 ${process.versions.v8}), wasm32-unknown-unknown release; one fresh process per run; warm is the median of ${WARM_RUNS} calls. Module compile and instantiation are not in the first call.`);
}
