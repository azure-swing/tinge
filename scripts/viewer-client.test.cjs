// Exercise the actual viewer client functions without a browser dependency.
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const test = require('node:test');
const source = readFileSync(join(__dirname, '../crates/cli/src/web/app.js'), 'utf8');

function snippet(start, end) {
  const first = source.indexOf(start);
  const last = source.indexOf(end, first);
  assert.ok(first >= 0 && last > first, `Missing client function boundary: ${start}`);
  return source.slice(first, last);
}

test('disconnect, timeout and reconnect preserve historical revision and drafts', async () => {
  const state = { connected: true, loading: true, generation: 3, pollBusy: false, project: { head: 4 }, revision: '2', shapes: [], dirty: false };
  let previewCount = 0;
  const context = vm.createContext({ state, token: 'test', AbortController, setTimeout, clearTimeout,
    fetch: async () => { throw new TypeError('fetch failed'); },
    controls() {}, updateHistory() {}, updateSelectionInfo() {}, drawOverlay() {}, toast() {}, status() {},
    requestPreview() { previewCount++; }
  });
  vm.runInContext(snippet('async function api(', 'let toastTimer;') + snippet('async function poll(', 'async function loadImage('), context);
  await assert.rejects(context.api('state'), e => e.connectionLost && /已断开/.test(e.message));
  await context.poll();
  assert.equal(state.connected, false);
  assert.equal(state.loading, false);
  assert.equal(state.generation, 4, 'disconnect invalidates the pending preview');
  await context.poll();
  assert.equal(state.generation, 4, 'repeated failures do not change the generation');
  context.fetch = async () => ({ ok: true, json: async () => ({ head: 4 }) });
  await context.poll();
  assert.equal(state.connected, true);
  assert.equal(state.revision, '2', 'reconnect preserves the historical revision');
  assert.equal(previewCount, 1, 'reconnect retries even when head is unchanged');
  state.connected = false; state.dirty = true; state.result = { revision: 2 }; state.shapes = ['draft'];
  await context.poll();
  assert.equal(previewCount, 1, 'reconnect preserves unsubmitted selections');
  assert.deepEqual(state.shapes, ['draft']);
  context.setTimeout = callback => setTimeout(callback, 5);
  context.fetch = async (_, { signal }) => ({ ok: true, json: () => new Promise((resolve, reject) => signal.addEventListener('abort', () => reject(new Error('aborted')))) });
  await assert.rejects(context.api('job/1'), e => e.connectionLost && /超时/.test(e.message));
  context.setTimeout = setTimeout;
  context.fetch = async () => ({ ok: false, json: async () => ({ error: { code: 'revision_conflict', actual_revision: 5 } }) });
  await assert.rejects(context.api('save', {}), e => !e.connectionLost && /v5/.test(e.message));
});

test('rapid choices submit one preview and display the latest revision', async () => {
  const state = { project: { head: 4, selections: [] }, connected: true, generation: 0, revision: '0', reference: 'original', maxEdge: 1600, drafts: new Map(), shapes: [], loading: false };
  const posts = [];
  const elements = new Map();
  const context = vm.createContext({ state, previewClient: 'test', sleep: ms => new Promise(resolve => setTimeout(resolve, ms)),
    currentRevision: () => Number(state.revision), revisionName: id => `v${id}`,
    controls() {}, status() {}, toast() {}, layout() {}, drawOverlay() {}, updateSelectionInfo() {}, loadImage: async () => {},
    document: { querySelector: () => ({ disabled: false }) },
    $: id => { if (!elements.has(id)) elements.set(id, { hidden: false, querySelector: () => ({ textContent: '' }) }); return elements.get(id); },
    api: async (path, body) => { if (path === 'preview') { posts.push(body); return { job: 1 }; } return { status: 'ready', result: { revision: posts.at(-1).revision, width: 10, height: 10, reference_width: 10, reference_height: 10, image: 'image/graded', reference: 'image/original', reference_revision: null } }; }
  });
  vm.runInContext(snippet('async function requestPreview(', 'function chooseRevision('), context);
  const pending = [];
  for (let i = 0; i < 25; i++) { state.revision = String(i % 5); pending.push(context.requestPreview()); }
  await Promise.all(pending);
  assert.equal(posts.length, 1, 'rapid choices submit only the final request');
  assert.equal(posts[0].revision, 4);
  assert.equal(state.result.revision, 4);
  assert.equal(state.loading, false);
});

test('restoring editable selections preserves their region through another save', () => {
  const context = vm.createContext({ state: { shapes: [] } });
  vm.runInContext(snippet('function selectionShapes(', 'function drawOverlay(') + snippet('function compiledMask(', 'function point('), context);
  const rectangle = (min, max) => ({ type: 'rectangle', min: [min, min], max: [max, max], feather: 0 });
  const alpha = (mask, x, y) => {
    if (mask.type === 'rectangle') return +(x >= mask.min[0] && x <= mask.max[0] && y >= mask.min[1] && y <= mask.max[1]);
    return mask.masks.slice(1).reduce((a, m) => mask.mode === 'union' ? 1 - (1 - a) * (1 - alpha(m, x, y)) : a * (1 - alpha(m, x, y)), alpha(mask.masks[0], x, y));
  };
  // These sequences include adding back a region removed by an earlier stroke.
  for (const modes of [['subtract', 'union', 'subtract'], ['union', 'subtract', 'union'], ['subtract', 'subtract', 'union']]) {
    let original = rectangle(0, 1);
    for (let i = 0; i < modes.length; i++) original = { type: 'combine', mode: modes[i], masks: [original, rectangle(.1 + i * .1, .9 - i * .1)] };
    context.state.shapes = context.selectionShapes(original);
    assert.equal(context.state.shapes.length, 4);
    const saved = context.compiledMask();
    for (let y = .025; y < 1; y += .05) for (let x = .025; x < 1; x += .05) assert.equal(alpha(saved, x, y), alpha(original, x, y));
  }
  const nary = { type: 'combine', mode: 'subtract', masks: [rectangle(0, 1), rectangle(.1, .3), rectangle(.7, .9)] };
  context.state.shapes = context.selectionShapes(nary);
  for (const p of [.2, .5, .8]) assert.equal(alpha(context.compiledMask(), p, p), alpha(nary, p, p));
});

test('grouped right operands and unsupported geometry never become editable strokes', () => {
  const context = vm.createContext({});
  vm.runInContext(snippet('function selectionShapes(', 'function drawOverlay('), context);
  const rectangle = (min, max) => ({ type: 'rectangle', min: [min, min], max: [max, max], feather: 0 });
  const a = rectangle(0, 1), b = rectangle(.2, .8), c = rectangle(.4, .6);
  for (const mode of ['union', 'subtract']) {
    const original = { type: 'combine', mode, masks: [a, { type: 'combine', mode: 'subtract', masks: [b, c] }] };
    const before = JSON.stringify(original);
    assert.equal(context.selectionShapes(original), null);
    assert.equal(JSON.stringify(original), before);
  }
  for (const mask of [{ ...a, feather: .1 }, { type: 'ellipse', center: [.5, .5], radius: [.2, .3], rotation: 10, feather: 0 }, { type: 'invert', mask: a }, { type: 'combine', mode: 'intersect', masks: [a, b] }, { type: 'combine', mode: 'union', masks: Array(13).fill(a) }]) assert.equal(context.selectionShapes(mask), null);
});

test('unsupported saved selection stays uneditable and receives a persistent explanation', async () => {
  const rectangle = (min, max) => ({ type: 'rectangle', min: [min, min], max: [max, max], feather: 0 });
  const mask = { type: 'combine', mode: 'subtract', masks: [rectangle(0, 1), { type: 'combine', mode: 'subtract', masks: [rectangle(.2, .8), rectangle(.4, .6)] }] };
  const selection = { revision: 4, width: 10, height: 10, mask, note: 'Keep the center' };
  const state = { project: { head: 4, selections: [selection] }, connected: true, generation: 0, revision: 'latest', reference: 'original', maxEdge: 1600, drafts: new Map(), shapes: [], loading: false };
  const elements = new Map();
  const context = vm.createContext({ state, previewClient: 'test', sleep: async () => {}, currentRevision: () => 4, revisionName: id => `v${id}`,
    controls() {}, status() {}, toast() {}, layout() {}, drawOverlay() {}, loadImage: async () => {},
    document: { querySelector: () => ({ disabled: false }) },
    $: id => { if (!elements.has(id)) elements.set(id, { querySelector: () => ({}) }); return elements.get(id); },
    api: async path => path === 'preview' ? { job: 1 } : { status: 'ready', result: { revision: 4, width: 10, height: 10, reference_width: 10, reference_height: 10, image: 'graded', reference: 'original', reference_revision: null } }
  });
  vm.runInContext(snippet('function selectionShapes(', 'function drawOverlay(') + snippet('function updateSelectionInfo(', 'async function poll(') + snippet('async function requestPreview(', 'function chooseRevision('), context);
  await context.requestPreview();
  assert.equal(state.loading, false);
  assert.equal(state.shapes.length, 0);
  assert.match(elements.get('selectionInfo').textContent, /此复杂选区暂不能在查看器中编辑/);
  assert.match(elements.get('selectionInfo').textContent, /Keep the center/);
  assert.equal(selection.mask, mask);
});
