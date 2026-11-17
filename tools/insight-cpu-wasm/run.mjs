// Times one insight step in V8, the engine Workers run, on the Worker's own
// wasm target. No dependency: Node's WebAssembly and a child process per
// measurement, so each first call pays a fresh module compile and every
// pattern's compile, as a cold isolate would.
//
//   node run.mjs WASM SNAPSHOT_DIR        all snapshots, a table and a summary
//   node run.mjs WASM SNAPSHOT --one      one snapshot (used by the above)
import { readFileSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const WARM_RUNS = 20;
const [wasmPath, target, flag] = process.argv.slice(2);

function measureOne(wasmPath, snapshotPath) {
  const bytes = readFileSync(wasmPath);
  const t0 = performance.now();
  const module = new WebAssembly.Module(bytes);
  // wasm-bindgen's imports (pulled in through chrono's default features)
  // are never called by the step: each is a stub that fails if it is.
  const imports = {};
  for (const i of WebAssembly.Module.imports(module)) {
    (imports[i.module] ??= {})[i.name] = () => {
      throw new Error(`unexpected call to ${i.module}.${i.name}`);
    };
  }
  const instance = new WebAssembly.Instance(module, imports);
  const instantiate = performance.now() - t0;
  const { memory, alloc, load, prepare, tick } = instance.exports;
  const snap = readFileSync(snapshotPath);
  const ptr = alloc(snap.length);
  new Uint8Array(memory.buffer, ptr, snap.length).set(snap);
  if (load(ptr, snap.length) !== 1) throw new Error(`bad snapshot ${snapshotPath}`);
  prepare();
  const t1 = performance.now();
  const compiled = tick();
  const first = performance.now() - t1;
  if (compiled >= 1_000_000) throw new Error(`the tick failed on ${snapshotPath}`);
  const warm = [];
  for (let i = 0; i < WARM_RUNS; i++) {
    prepare();
    const t = performance.now();
    tick();
    warm.push(performance.now() - t);
  }
  warm.sort((a, b) => a - b);
  return { first, warm: warm[Math.floor(warm.length / 2)], compiled, instantiate };
}

if (flag === "--one") {
  console.log(JSON.stringify(measureOne(wasmPath, target)));
} else {
  const files = readdirSync(target).filter((f) => f.endsWith(".json")).sort();
  const rows = {};
  for (const f of files) {
    const [post, step] = f.replace(/\.json$/, "").split(".");
    const out = execFileSync(process.execPath, [process.argv[1], wasmPath, join(target, f), "--one"]);
    const r = JSON.parse(out.toString());
    (rows[step] ??= []).push({ post, ...r });
    console.log(`${post} ${step}: first ${r.first.toFixed(2)} ms, warm ${r.warm.toFixed(2)} ms, ${r.compiled} patterns`);
  }
  const stat = (v) => {
    const s = [...v].sort((a, b) => a - b);
    return { worst: s[s.length - 1], median: s[Math.floor(s.length / 2)] };
  };
  console.log("\n| Step | Posts | First call, worst | First call, median | Warm, worst | Warm, median | Patterns compiled |");
  console.log("|---|---|---|---|---|---|---|");
  for (const [step, rs] of Object.entries(rows)) {
    const f = stat(rs.map((r) => r.first));
    const w = stat(rs.map((r) => r.warm));
    const p = rs.map((r) => r.compiled);
    console.log(
      `| ${step.replace(/^\d-/, "")} | ${rs.length} | ${f.worst.toFixed(2)} ms | ${f.median.toFixed(2)} ms | ${w.worst.toFixed(2)} ms | ${w.median.toFixed(2)} ms | ${Math.min(...p)}-${Math.max(...p)} |`,
    );
  }
  console.log(`\nNode ${process.version} (V8 ${process.versions.v8}), wasm32-unknown-unknown release; warm is the median of ${WARM_RUNS} runs. Module compile and instantiation are not in the first call.`);
}
