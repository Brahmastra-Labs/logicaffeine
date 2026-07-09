// WS5 (work/FINISH_INTERPRETER.md Phase 12) — drive the true-multicore WASM threads primitive.
//
// One file, two roles (node `worker_threads`, real OS threads — the headless analog of browser
// Web Workers). The MAIN role creates ONE shared `WebAssembly.Memory`, spawns N workers sharing
// it, and after they finish checks the contended counter == N*iters and that every worker
// cleared the concurrency barrier. Each WORKER instantiates the same module against the shared
// memory and runs `worker_main` — they only pass the barrier if they run at the same time, which
// is exactly what "true multicore" means.
//
// Usage:  node run.mjs <worker_probe.wasm> [numWorkers] [iters]

import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import { readFileSync } from 'node:fs';

const INITIAL_PAGES = 32;   // 2 MiB — must match rustc's --initial-memory
const MAX_PAGES = 256;      // 16 MiB — must match rustc's --max-memory

if (isMainThread) {
  const wasmPath = process.argv[2];
  const numWorkers = parseInt(process.argv[3] ?? '4', 10);
  const iters = parseInt(process.argv[4] ?? '1000000', 10);
  if (!wasmPath) {
    console.error('usage: node run.mjs <worker_probe.wasm> [numWorkers] [iters]');
    process.exit(2);
  }

  const bytes = readFileSync(wasmPath);
  const memory = new WebAssembly.Memory({ initial: INITIAL_PAGES, maximum: MAX_PAGES, shared: true });
  if (!(memory.buffer instanceof SharedArrayBuffer)) {
    console.error('FAIL: WebAssembly.Memory is not backed by a SharedArrayBuffer — no shared memory');
    process.exit(1);
  }
  const mod = await WebAssembly.compile(bytes);

  const results = await Promise.all(
    Array.from({ length: numWorkers }, (_, tid) =>
      new Promise((resolve, reject) => {
        const w = new Worker(new URL(import.meta.url), {
          workerData: { mod, memory, tid, numWorkers, iters },
        });
        const timer = setTimeout(() => {
          w.terminate();
          reject(new Error(`worker ${tid} timed out — barrier never released (no true concurrency?)`));
        }, 60_000);
        w.on('message', (rc) => { clearTimeout(timer); resolve(rc); });
        w.on('error', (e) => { clearTimeout(timer); reject(e); });
      })
    )
  );

  // Read the final counter from a fresh instance on the shared memory.
  const checker = await WebAssembly.instantiate(mod, { env: { memory } });
  const counter = checker.exports.get_counter();

  const expected = numWorkers * iters;
  const allCleared = results.every((rc) => rc === 0);
  const ok = allCleared && counter === expected;

  console.log(`workers=${numWorkers} iters=${iters}`);
  console.log(`  barrier: ${allCleared ? 'all cleared (true concurrency)' : `FAILED: ${JSON.stringify(results)}`}`);
  console.log(`  counter: ${counter} (expected ${expected}) ${counter === expected ? 'OK' : 'MISMATCH'}`);
  console.log(ok ? 'PASS: genuine multicore WebAssembly over shared memory' : 'FAIL');
  process.exit(ok ? 0 : 1);
} else {
  const { mod, memory, tid, numWorkers, iters } = workerData;
  const instance = await WebAssembly.instantiate(mod, { env: { memory } });
  // Defensive per-worker stack region (harmless if the leaf entry uses no linear stack).
  if (instance.exports.__stack_pointer) {
    instance.exports.__stack_pointer.value = 0x18_0000 - tid * 0x2_0000;
  }
  const rc = instance.exports.worker_main(numWorkers, iters);
  parentPort.postMessage(rc);
}
