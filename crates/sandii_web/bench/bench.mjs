#!/usr/bin/env node
// Headless frame-time benchmark for the Sandii web build.
//
//   cd crates/sandii_web && trunk build --release && node bench/bench.mjs
//
// Serves dist/ (or --dist DIR) on a local port, opens it in headless Chromium at a phone-sized
// viewport with a CPU throttle (default 4x, a stand-in for a mid-range phone), fills the screen
// with each scene through the page's bench hooks (bench_fill / bench_stats in src/lib.rs) and
// prints ms per frame split into simulation and render.
//
// Options: --dist DIR  --throttle N  --seconds S  --scenes water,lava,mix,rain
//          --viewport 844x390  --runs N (repeat each scene, report the median)  --json
// Playwright: resolved from this package, NODE_PATH, or $PLAYWRIGHT (path to the package).
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { extname, join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => {
    if (a.startsWith('--')) acc.push([a.slice(2), all[i + 1]?.startsWith('--') || all[i + 1] === undefined ? 'true' : all[i + 1]]);
    return acc;
  }, []),
);
const dist = resolve(args.dist || join(here, '..', 'dist'));
const throttle = Number(args.throttle || 4);
const seconds = Number(args.seconds || 4);
const runs = Number(args.runs || 1);
const scenes = (args.scenes || 'water,lava,mix,rain').split(',');
const [vw, vh] = (args.viewport || '844x390').split('x').map(Number);

function loadPlaywright() {
  const req = createRequire(import.meta.url);
  const tries = [process.env.PLAYWRIGHT, 'playwright', 'playwright-core'].filter(Boolean);
  for (const t of tries) {
    try { return req(t); } catch {}
  }
  throw new Error('playwright not found: npm i playwright, or set PLAYWRIGHT=/path/to/node_modules/playwright');
}

const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.png': 'image/png' };
const server = createServer(async (req, res) => {
  const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = join(dist, path.endsWith('/') ? path + 'index.html' : path);
  try {
    const body = await readFile(file);
    res.writeHead(200, { 'content-type': MIME[extname(file)] || 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404); res.end();
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const { chromium } = loadPlaywright();
const browser = await chromium.launch({ headless: true });

async function runScene(scene) {
  const ctx = await browser.newContext({ viewport: { width: vw, height: vh }, deviceScaleFactor: 3, isMobile: true, hasTouch: true });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);
  await page.waitForFunction(() => window.wasmBindings && window.wasmBindings.bench_stats, null, { timeout: 30000 });
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate: throttle });
  await sleep(800);
  const ok = await page.evaluate((s) => window.wasmBindings.bench_fill(s), scene);
  if (!ok) throw new Error(`unknown scene ${scene}`);
  await sleep(1500); // warm up: let the fill settle into steady state
  await page.evaluate(() => window.wasmBindings.bench_reset());
  await sleep(seconds * 1000);
  const s = await page.evaluate(() => Array.from(window.wasmBindings.bench_stats()));
  await ctx.close();
  if (errors.length) throw new Error(`${scene}: ${errors.join('; ')}`);
  const [frames, frameMs, gaps, maxFrame, simMs, renderMs, substeps, gw, gh] = s;
  return {
    scene,
    grid: `${gw}x${gh}`,
    fps: (gaps / frameMs) * 1000,
    frame_ms: frameMs / gaps,
    max_frame_ms: maxFrame,
    sim_ms: simMs / frames,
    render_ms: renderMs / frames,
    steps_per_frame: substeps / frames,
    ms_per_step: substeps ? simMs / substeps : 0,
  };
}

const median = (xs) => xs.slice().sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const results = [];
for (const scene of scenes) {
  const rs = [];
  for (let i = 0; i < runs; i++) rs.push(await runScene(scene));
  const pick = rs.slice().sort((a, b) => a.frame_ms - b.frame_ms)[Math.floor(rs.length / 2)];
  results.push({ ...pick, runs: rs.length, fps_runs: rs.map((r) => +r.fps.toFixed(1)) });
}
await browser.close();
server.close();

if (args.json) {
  console.log(JSON.stringify({ throttle, viewport: `${vw}x${vh}`, seconds, results }, null, 2));
} else {
  console.log(`Sandii bench: ${dist}\nviewport ${vw}x${vh} css px, CPU throttle ${throttle}x, ${seconds}s per scene, ${runs} run(s)\n`);
  const f = (n, d = 1) => n.toFixed(d).padStart(7);
  console.log('scene    grid       fps  frame ms    max ms    sim ms render ms  steps/fr  ms/step');
  for (const r of results) {
    console.log(`${r.scene.padEnd(8)} ${r.grid.padEnd(8)} ${f(r.fps)} ${f(r.frame_ms, 2)}   ${f(r.max_frame_ms)}   ${f(r.sim_ms, 2)}   ${f(r.render_ms, 2)}   ${f(r.steps_per_frame, 2)}  ${f(r.ms_per_step, 2)}`);
  }
}
