'use strict';
const $ = id => document.getElementById(id);
const token = location.pathname.split('/')[1];
const previewClient = crypto.randomUUID();
const state = { project: null, connected: true, revision: 'latest', reference: 'original', mode: 'graded', tool: 'hand', selectionMode: 'add', shapes: [], drawing: null, dirty: false, drafts: new Map(), result: null, job: null, generation: 0, loading: false, zoom: null, pan: [0, 0], maxEdge: 1600, historyKey: '', pollBusy: false };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function api(path, body) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15000);
  let response, value;
  try {
    response = await fetch(`api/${path}`, { ...(body === undefined ? { cache: 'no-store' } : { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-VibeColor': token }, body: JSON.stringify(body) }), signal: controller.signal });
    value = await response.json();
  } catch (cause) {
    const error = new Error(controller.signal.aborted ? '本地引擎响应超时，请检查预览服务。保存操作请先核对版本记录。' : '本地预览服务已断开，请重新启动服务并打开新的预览地址。');
    error.connectionLost = true;
    throw error;
  } finally { clearTimeout(timer); }
  if (!response.ok || value.ok === false) {
    const error = value.error;
    throw new Error(error?.code === 'revision_conflict' ? `项目已经更新到 v${error.actual_revision}。请核对要保存的版本后再试；本次操作未修改数据。` : (error?.message || '本地引擎返回错误'));
  }
  return value;
}
let toastTimer;
function toast(message) { $('toast').textContent = message; $('toast').hidden = false; clearTimeout(toastTimer); toastTimer = setTimeout(() => { $('toast').hidden = true; }, 5000); }
function status(message, kind = '') { $('status').textContent = message; $('statusDot').className = `dot ${kind}`; }
function currentRevision() { return state.revision === 'latest' ? state.project?.head : Number(state.revision); }
function revisionName(id) { const names = Object.entries(state.project?.tags || {}).filter(([, target]) => target === id).map(([name]) => name); return names.length ? `${names.join(' · ')} · v${id}` : `v${id}`; }
function controls() {
  $('selectionButton').disabled = !state.connected || !state.shapes.length || state.loading || !state.result;
  $('saveButton').disabled = !state.connected || !state.result || state.loading;
  $('finalizeButton').disabled = !state.connected || !state.result || state.loading;
  $('restoreButton').disabled = !state.connected || !state.result || state.loading || state.result.revision === state.project?.head;
  $('divider').hidden = state.mode !== 'wipe';
  $('stage').dataset.mode = state.mode; $('stage').dataset.tool = state.tool;
  document.querySelectorAll('[data-mode]').forEach(button => { if (button.tagName === 'BUTTON') { button.classList.toggle('active', button.dataset.mode === state.mode); button.setAttribute('aria-pressed', button.dataset.mode === state.mode); } });
  document.querySelectorAll('[data-tool]').forEach(button => { if (button.tagName === 'BUTTON') { button.classList.toggle('active', button.dataset.tool === state.tool); button.setAttribute('aria-pressed', button.dataset.tool === state.tool); } });
  document.querySelectorAll('[data-selection]').forEach(button => { button.classList.toggle('active', button.dataset.selection === state.selectionMode); button.setAttribute('aria-pressed', button.dataset.selection === state.selectionMode); });
}
function updateHistory() {
  const p = state.project;
  const key = JSON.stringify([p.head, p.history, p.tags, p.final_revision, state.revision, state.reference]);
  if (key === state.historyKey) return;
  state.historyKey = key; $('projectName').textContent = p.name; $('branch').textContent = p.final_revision === null ? `${p.branch} · agent 的修改会自动显示` : `已定稿 · v${p.final_revision}`;
  const revision = $('revision'); revision.replaceChildren(new Option(`跟随最新 · v${p.head}`, 'latest'));
  const reference = $('reference'); reference.replaceChildren(new Option('原图', 'original'));
  const history = $('history'); history.replaceChildren();
  p.history.forEach(r => {
    const name = `${revisionName(r.id)} · ${r.label}`;
    revision.add(new Option(name, r.id)); reference.add(new Option(name, r.id));
    const button = document.createElement('button'); button.className = `historyItem${r.id === currentRevision() ? ' current' : ''}`; button.dataset.revision = r.id;
    const label = document.createElement('strong'); label.textContent = revisionName(r.id);
    const detail = document.createElement('small'); detail.textContent = `${r.label} · ${new Date(r.timestamp * 1000).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })}`;
    button.append(label, detail); button.addEventListener('click', () => chooseRevision(String(r.id))); history.append(button);
  });
  revision.value = state.revision; reference.value = state.reference;
  if (state.result) { $('gradedLabel').textContent = `效果 · ${revisionName(state.result.revision)}`; $('referenceLabel').textContent = state.result.reference_revision === null ? '原图 · 当前显影 / 显示变换' : `对比 · ${revisionName(state.result.reference_revision)}`; }
  controls();
}
function updateSelectionInfo() {
  const item = state.project?.selections.find(s => s.revision === state.result?.revision);
  $('selectionInfo').textContent = item ? `已保存 · ${revisionName(item.revision)} · ${item.width} × ${item.height}${item.note ? `\n${item.note}` : ''}` : '圈出想要调整的区域，再保存圈选。';
}
async function poll() {
  if (state.pollBusy) return;
  state.pollBusy = true;
  try {
    const p = await api('state'); const oldHead = state.project?.head; const reconnected = !state.connected; state.connected = true; state.project = p;
    updateHistory(); updateSelectionInfo();
    if (oldHead === undefined || reconnected || (p.head !== oldHead && state.revision === 'latest')) {
      if (state.dirty && state.result) {
        state.revision = String(state.result.revision); updateHistory();
        status(`有新版本 v${p.head} · 当前保留 v${state.result.revision} 的圈划`);
        toast('有新版本了。当前画面保留未提交的圈划，可保存后切到最新版本。');
      } else { state.shapes = []; state.drawing = null; state.dirty = false; drawOverlay(); requestPreview(); }
    }
  } catch (e) { if (e.connectionLost) { if (state.connected) state.generation++; state.connected = false; state.loading = false; controls(); } status(e.message, 'error'); }
  finally { state.pollBusy = false; }
}
async function loadImage(src) { const img = new Image(); img.src = src; await img.decode(); return img; }
async function requestPreview() {
  if (!state.project || !state.connected) return;
  const generation = ++state.generation; state.loading = true; state.drawing = null; controls();
  const requestedRevision = currentRevision(); const requestedReference = state.reference;
  status('正在生成预览', 'busy');
  try {
    // Coalesce rapid choices before asking the native engine to do expensive work.
    await sleep(120);
    if (generation !== state.generation) return;
    const { job } = await api('preview', { revision: requestedRevision, reference: requestedReference === 'original' ? null : Number(requestedReference), max_edge: state.maxEdge, client_id: previewClient });
    let result;
    while (generation === state.generation) {
      const progress = await api(`job/${job}`);
      if (progress.status === 'error') throw new Error(progress.error?.message || '预览失败');
      if (progress.status === 'cancelled') throw new Error('已取消过时的预览，请重新选择版本。');
      if (progress.status === 'ready') { result = progress.result; break; }
      const nodes = progress.progress; status(nodes ? `${progress.phase} · ${nodes.completed}/${nodes.total}` : (progress.phase || '等待本地引擎'), 'busy');
      await sleep(200);
    }
    if (generation !== state.generation || !result) return;
    await Promise.all([loadImage(result.image), loadImage(result.reference)]);
    if (generation !== state.generation) return;
    state.result = result; state.job = job; $('gradedImage').src = result.image; $('referenceImage').src = result.reference;
    $('stage').hidden = false; $('empty').hidden = true;
    const sameGeometry = result.width === result.reference_width && result.height === result.reference_height;
    document.querySelector('[data-mode="wipe"]').disabled = !sameGeometry;
    if (!sameGeometry && state.mode === 'wipe') { state.mode = 'side'; toast('两张图片尺寸不同，已切换为并排对比。'); }
    $('dimensions').textContent = `${result.width} × ${result.height}`;
    $('gradedLabel').textContent = `效果 · ${revisionName(result.revision)}`;
    $('referenceLabel').textContent = result.reference_revision === null ? '原图 · 当前显影 / 显示变换' : `对比 · ${revisionName(result.reference_revision)}`;
    const selection = state.project.selections.find(s => s.revision === result.revision);
    const draft = state.drafts.get(result.revision);
    if (draft && !state.shapes.length) { state.shapes = structuredClone(draft.shapes); $('note').value = draft.note; state.dirty = true; }
    else if (selection && !state.shapes.length) { const restored = flattenMask(selection.mask); if (restored.length <= 12 && restored.every(s => ['ellipse', 'polygon', 'rectangle'].includes(s.mask.type) && s.mask.feather === 0 && !s.mask.rotation)) state.shapes = restored; $('note').value = selection.note; state.dirty = false; }
    controls(); layout(); drawOverlay(); updateSelectionInfo();
    status(`已同步 · ${revisionName(result.revision)}${state.revision === 'latest' ? ' · 跟随 agent' : ' · 历史版本'}`);
  } catch (e) { if (generation === state.generation) { status(e.message, 'error'); toast(e.message); if (!state.result) $('empty').querySelector('strong').textContent = '预览暂时无法生成'; } }
  finally { if (generation === state.generation) { state.loading = false; controls(); } }
}
function chooseRevision(revision) { if (state.dirty && state.result) state.drafts.set(state.result.revision, { shapes: structuredClone(state.shapes), note: $('note').value }); state.revision = revision; state.shapes = []; state.drawing = null; state.dirty = false; state.zoom = null; state.pan = [0, 0]; state.maxEdge = 1600; $('note').value = ''; updateHistory(); drawOverlay(); requestPreview(); }
function layout() {
  if (!state.result) return;
  const area = $('stage').getBoundingClientRect(); const r = state.result;
  const paneWidth = area.width / (state.mode === 'side' ? 2 : 1);
  const scale = state.zoom ?? Math.min((paneWidth - 40) / Math.max(r.width, r.reference_width), (area.height - 50) / Math.max(r.height, r.reference_height));
  state.displayScale = Math.max(0.001, scale);
  for (const [id, w, h] of [['gradedPhoto', r.width, r.height], ['referencePhoto', r.reference_width, r.reference_height]]) {
    const photo = $(id); const width = w * state.displayScale, height = h * state.displayScale;
    photo.style.width = `${width}px`; photo.style.height = `${height}px`; photo.style.left = `${(paneWidth - width) / 2 + state.pan[0]}px`; photo.style.top = `${(area.height - height) / 2 + state.pan[1]}px`;
  }
  $('fitButton').classList.toggle('active', state.zoom === null);
  $('pixelButton').classList.toggle('active', state.zoom !== null && Math.abs(state.zoom * devicePixelRatio - 1) < 0.001);
}
const ns = 'http://www.w3.org/2000/svg';
function svg(tag, attrs = {}) { const node = document.createElementNS(ns, tag); Object.entries(attrs).forEach(([k, v]) => node.setAttribute(k, v)); return node; }
function geometry(mask, fill) {
  switch (mask.type) {
    case 'ellipse': return svg('ellipse', { cx: mask.center[0], cy: mask.center[1], rx: mask.radius[0], ry: mask.radius[1], fill });
    case 'polygon': return svg('polygon', { points: mask.points.map(p => p.join(',')).join(' '), fill });
    case 'rectangle': return svg('rect', { x: mask.min[0], y: mask.min[1], width: mask.max[0] - mask.min[0], height: mask.max[1] - mask.min[1], fill });
    default: return svg('g');
  }
}
function flattenMask(mask, mode = 'add') {
  if (mask.type === 'combine' && ['union', 'subtract'].includes(mask.mode)) return mask.masks.flatMap((m, i) => flattenMask(m, mask.mode === 'subtract' && i > 0 ? 'subtract' : mode));
  return [{ mode, mask }];
}
function drawOverlay() {
  const overlay = $('overlay'); overlay.replaceChildren();
  const shapes = state.shapes.flatMap(s => flattenMask(s.mask, s.mode));
  if (state.drawing?.mask) shapes.push({ mode: state.selectionMode, mask: state.drawing.mask });
  const defs = svg('defs'); const mask = svg('mask', { id: 'selectionArea', maskUnits: 'userSpaceOnUse', x: 0, y: 0, width: 1, height: 1, 'mask-type': 'luminance' });
  mask.append(svg('rect', { width: 1, height: 1, fill: 'black' }));
  shapes.forEach(s => mask.append(geometry(s.mask, s.mode === 'add' ? 'white' : 'black'))); defs.append(mask); overlay.append(defs);
  overlay.append(svg('rect', { width: 1, height: 1, fill: '#c6b7fa', 'fill-opacity': .23, mask: 'url(#selectionArea)' }));
  shapes.forEach(s => { const outline = geometry(s.mask, 'none'); outline.setAttribute('stroke', s.mode === 'add' ? '#e0d6ff' : '#ffad91'); outline.setAttribute('stroke-width', '1.5'); outline.setAttribute('vector-effect', 'non-scaling-stroke'); if (s.mode === 'subtract') outline.setAttribute('stroke-dasharray', '4 3'); overlay.append(outline); });
  controls();
}
function compiledMask() {
  let mask = null;
  for (const shape of state.shapes) {
    if (!mask) { if (shape.mode === 'subtract') throw new Error('请先圈出一个区域，再减去区域。'); mask = shape.mask; }
    else mask = { type: 'combine', mode: shape.mode === 'add' ? 'union' : 'subtract', masks: [mask, shape.mask] };
  }
  return mask;
}
function point(event) { const rect = $('gradedPhoto').getBoundingClientRect(); return [(event.clientX - rect.left) / rect.width, (event.clientY - rect.top) / rect.height]; }
function clamp(p) { return p.map(v => Math.max(0, Math.min(1, v))); }
let gesture = null;
$('stage').addEventListener('pointerdown', event => {
  if (!state.result || state.loading || event.button !== 0) return;
  if (event.target.closest('#divider')) gesture = { type: 'wipe' };
  else if (state.tool === 'hand' || event.altKey) gesture = { type: 'pan', start: [event.clientX, event.clientY], pan: [...state.pan] };
  else {
    if (state.shapes.length >= 12) return toast('一次圈选最多 12 个区域，请先保存。');
    if (state.selectionMode === 'subtract' && !state.shapes.length) return toast('先圈出区域，再减去不需要的部分。');
    const p = point(event); if (p.some(v => v < 0 || v > 1)) return;
    if (state.mode === 'side' && event.clientX < $('gradedPhoto').parentElement.getBoundingClientRect().left) return;
    state.drawing = { points: [p], start: p, mask: null }; gesture = { type: 'draw' };
  }
  event.preventDefault(); $('stage').setPointerCapture(event.pointerId);
});
$('stage').addEventListener('pointermove', event => {
  if (!gesture) return;
  if (gesture.type === 'wipe') { const rect = $('stage').getBoundingClientRect(); const percent = Math.max(0, Math.min(100, (event.clientX - rect.left) / rect.width * 100)); $('stage').style.setProperty('--wipe', `${percent}%`); $('divider').style.left = `${percent}%`; }
  else if (gesture.type === 'pan') { state.pan = [gesture.pan[0] + event.clientX - gesture.start[0], gesture.pan[1] + event.clientY - gesture.start[1]]; layout(); }
  else if (state.drawing) {
    const p = clamp(point(event)), drawing = state.drawing;
    if (state.tool === 'ellipse') { const radius = [Math.abs(p[0] - drawing.start[0]) / 2, Math.abs(p[1] - drawing.start[1]) / 2]; drawing.mask = { type: 'ellipse', center: [(p[0] + drawing.start[0]) / 2, (p[1] + drawing.start[1]) / 2], radius, rotation: 0, feather: 0 }; }
    else { const last = drawing.points.at(-1); if (Math.hypot(p[0] - last[0], p[1] - last[1]) > .002 && drawing.points.length < 4096) drawing.points.push(p); drawing.mask = { type: 'polygon', points: drawing.points, feather: 0 }; }
    drawOverlay();
  }
});
function endGesture(event) {
  if (gesture?.type === 'draw' && state.drawing) {
    const mask = state.drawing.mask;
    const area = mask?.type === 'polygon' ? Math.abs(mask.points.reduce((sum, p, i, points) => { const next = points[(i + 1) % points.length]; return sum + p[0] * next[1] - next[0] * p[1]; }, 0)) / 2 : 0;
    if (mask && ((mask.type === 'polygon' && mask.points.length >= 3 && area > .000001) || (mask.type === 'ellipse' && mask.radius.every(v => v >= .00001)))) { state.shapes.push({ mode: state.selectionMode, mask }); state.dirty = true; }
    state.drawing = null; drawOverlay();
  }
  gesture = null; if ($('stage').hasPointerCapture(event.pointerId)) $('stage').releasePointerCapture(event.pointerId);
}
$('stage').addEventListener('pointerup', endGesture);
$('stage').addEventListener('pointercancel', () => { state.drawing = null; gesture = null; drawOverlay(); });
$('stage').addEventListener('wheel', event => { if (!state.result) return; event.preventDefault(); state.zoom = Math.min(8 / devicePixelRatio, Math.max(.001, state.displayScale * Math.exp(-event.deltaY * .0015))); layout(); }, { passive: false });
$('modes').addEventListener('click', event => { const button = event.target.closest('button[data-mode]'); if (!button || button.disabled) return; state.mode = button.dataset.mode; if (state.mode === 'original' || state.mode === 'wipe') state.tool = 'hand'; controls(); layout(); });
$('tools').addEventListener('click', event => { const tool = event.target.closest('[data-tool]')?.dataset.tool; if (!tool) return; state.tool = tool; if (tool !== 'hand' && ['original', 'wipe'].includes(state.mode)) state.mode = 'graded'; controls(); layout(); });
$('selectionModes').addEventListener('click', event => { const mode = event.target.closest('[data-selection]')?.dataset.selection; if (mode) { state.selectionMode = mode; controls(); } });
$('clearButton').addEventListener('click', () => { state.shapes = []; state.drawing = null; state.dirty = false; if (state.result) state.drafts.delete(state.result.revision); drawOverlay(); });
$('note').addEventListener('input', () => { if (state.shapes.length) state.dirty = true; });
$('selectionButton').addEventListener('click', async () => {
  if (state.loading || !state.result || !state.shapes.length) return;
  $('selectionButton').disabled = true;
  try { const saved = await api('selection', { expect_revision: state.project.head, job: state.job, mask: compiledMask(), note: $('note').value }); state.dirty = false; state.drafts.delete(saved.selection.revision); toast(`圈选已保存 · ${revisionName(saved.selection.revision)}，agent 可以读取。`); await poll(); }
  catch (e) { toast(e.message); } finally { controls(); }
});
function showVersions(show) { $('versions').hidden = !show; $('versionsButton').setAttribute('aria-expanded', show); layout(); }
$('versionsButton').addEventListener('click', () => showVersions($('versions').hidden)); $('closeVersions').addEventListener('click', () => showVersions(false));
$('revision').addEventListener('change', event => chooseRevision(event.target.value));
$('reference').addEventListener('change', event => { state.reference = event.target.value; updateHistory(); requestPreview(); });
$('fitButton').addEventListener('click', () => { state.zoom = null; state.pan = [0, 0]; layout(); });
$('pixelButton').addEventListener('click', () => { state.zoom = 1 / devicePixelRatio; state.pan = [0, 0]; layout(); if (state.result && Math.max(state.result.width, state.result.height, state.result.reference_width, state.result.reference_height) > 16384) toast('图片超过原尺寸预览上限，当前为放大预览。'); if (state.maxEdge !== 16384) { state.maxEdge = 16384; requestPreview(); } });
$('refreshButton').addEventListener('click', async () => { await poll(); requestPreview(); });
$('saveButton').addEventListener('click', () => { state.saveTarget = { expect_revision: state.project.head, revision: state.result.revision }; $('saveDialog').querySelector('form > p').textContent = `给 ${revisionName(state.saveTarget.revision)} 起个名字，随时回来对比。`; $('saveError').textContent = ''; $('versionName').value = ''; $('saveDialog').showModal(); $('versionName').focus(); });
$('cancelSave').addEventListener('click', () => $('saveDialog').close());
$('saveForm').addEventListener('submit', async event => { event.preventDefault(); const button = event.submitter; button.disabled = true; try { await api('save', { ...state.saveTarget, name: $('versionName').value }); $('saveDialog').close(); toast('版本已保存。'); await poll(); } catch (e) { $('saveError').textContent = e.message; } finally { button.disabled = false; } });
$('finalizeButton').addEventListener('click', async () => {
  state.finalizeTarget = { expect_revision: state.project.head, revision: state.result.revision };
  const target = state.finalizeTarget;
  $('finalizeError').textContent = ''; $('finalizeSummary').textContent = '正在计算可回收的临时文件…';
  $('finalizeDialog').showModal(); $('finalizeForm').querySelector('[type="submit"]').disabled = true;
  try { const plan = await api(`cleanup-plan/${target.revision}`); if (target !== state.finalizeTarget || !$('finalizeDialog').open) return; $('finalizeSummary').textContent = `最终版本：${revisionName(target.revision)}。可回收 ${plan.files.length} 个临时文件，共 ${(plan.bytes / 1048576).toFixed(1)} MB${plan.skipped.length ? `；${plan.skipped.length} 个已修改或不可清理的文件会跳过` : ''}。`; $('finalizeForm').querySelector('[type="submit"]').disabled = false; }
  catch (error) { if (target === state.finalizeTarget && $('finalizeDialog').open) $('finalizeError').textContent = error.message; }
});
$('cancelFinalize').addEventListener('click', () => $('finalizeDialog').close());
$('finalizeForm').addEventListener('submit', async event => {
  event.preventDefault(); const button = event.submitter; button.disabled = true;
  try { const result = await api('finalize', state.finalizeTarget); await poll();
    const message = `v${result.revision} 已定稿，${result.recycled.length} 个临时文件已移到回收站。`;
    if (result.failed.length || result.registry_error) { $('finalizeError').textContent = `${message} ${result.failed.length} 个文件暂未回收，可重试；源图和历史仍保留。${result.registry_error || ''}`; }
    else { $('finalizeDialog').close(); toast(message); }
  } catch (error) { $('finalizeError').textContent = error.message; } finally { button.disabled = false; }
});
$('restoreButton').addEventListener('click', async () => { if (!state.result) return; $('restoreButton').disabled = true; try { await api('restore', { expect_revision: state.project.head, revision: state.result.revision }); state.revision = 'latest'; state.shapes = []; state.historyKey = ''; toast('已恢复，并保留全部历史版本。'); await poll(); } catch (e) { toast(e.message); } finally { controls(); } });
window.addEventListener('keydown', event => { if (event.target.matches('input,textarea,select') || $('saveDialog').open || $('finalizeDialog').open) return; if (event.key === 'Escape') { state.drawing = null; gesture = null; drawOverlay(); } if (event.key.toLowerCase() === 'f') $('fitButton').click(); if (event.key === '1') $('pixelButton').click(); if (event.key.toLowerCase() === 'b') { state.mode = state.mode === 'original' ? 'graded' : 'original'; state.tool = 'hand'; controls(); layout(); } });
new ResizeObserver(layout).observe($('workspace'));
const gamut = matchMedia('(color-gamut: p3)').matches ? 'P3' : (matchMedia('(color-gamut: srgb)').matches ? 'sRGB' : '未知');
$('color').title = `预览：sRGB SDR、8 位、ICC 标签。浏览器报告色域：${gamut}，HDR 能力：${matchMedia('(dynamic-range: high)').matches ? '有' : '未报告'}。能力检测不代表校准，也不改变预览颜色。正式导出由 Rust 色彩管线处理。`;
controls(); poll(); setInterval(poll, 1200);
