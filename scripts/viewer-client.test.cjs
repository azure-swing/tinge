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
