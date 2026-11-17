# insight-cpu-wasm

The insight step's CPU in V8, the engine Cloudflare Workers run, on the
Worker's own target (`wasm32-unknown-unknown`), under Node. Needs no
toolchain beyond the one the Worker builds with, and makes no model call.

`insight-trial time --dump DIR` writes one snapshot per step of each stored
brief and recap: the store before the step and the stored replies. This
crate is a `cdylib` with `load`, `prepare` and `tick`: `tick` runs
`insight::step::insight_tick` once from the snapshot. `run.mjs` runs each
snapshot in its own Node process and times the first `tick` (a fresh module:
V8 compiles the code it runs and every pattern compiles, as in a cold
isolate) and the median of 20 more. Module compile and instantiation are
outside the timed call.

From the repository root:

```sh
cd tools/insight-trial
env -u CARGO_TARGET_DIR cargo run --release -- time --facts facts --lines out/lines.jsonl --dump ../insight-cpu-wasm/snapshots
cd ../insight-cpu-wasm
env -u CARGO_TARGET_DIR cargo build --release --target wasm32-unknown-unknown
node run.mjs target/wasm32-unknown-unknown/release/insight_cpu_wasm.wasm snapshots
```

`run.mjs` also times calibration cases the same way, 13 fresh processes
each: a no-op call into the module (the fixed cost of a first call), the
insight tick with nothing to do, and two paths that already run in
production within the free plan: a share card render (`cover::card`) from
the real cover of the recap of 2026-11-06 (`calibration/*.cover.svg`,
written by `cover-graphic` from the committed facts), and the number check a
recap build runs on its draft (`numbers::check` against the facts' numbers)
on that recap's published English body (`calibration/*.en.md`, read from
D1). Read the insight steps' cold figures against those.

Node's V8 is not the Workers runtime: a Worker's isolate may already have
compiled code an earlier message ran, and Cloudflare measures CPU its own
way. This is a closer estimate than native time, not a measurement on
Workers.
