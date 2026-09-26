import {createHash} from 'node:crypto';
import {createReadStream} from 'node:fs';
import {readFile, writeFile} from 'node:fs/promises';
import {createInterface} from 'node:readline';
import {resolve, join} from 'node:path';
import {pathToFileURL} from 'node:url';
import assert from 'node:assert/strict';

const configurations = ['original', 'stable-full', 'stable-selected', 'dynamic-full', 'dynamic-selected'];
const equal = (left, right, label) => assert.deepEqual(left, right, label);
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const json = async path => JSON.parse(await readFile(path, 'utf8'));
async function hashFile(path) {
  const hash = createHash('sha256');
  let bytes = 0;
  for await (const chunk of createReadStream(path)) { hash.update(chunk); bytes += chunk.length; }
  return {bytes, sha256: hash.digest('hex')};
}
async function* frames(path) {
  for await (const line of createInterface({input: createReadStream(path), crlfDelay: Infinity})) {
    assert(line.trim(), 'empty frame row');
    yield JSON.parse(line);
  }
}
export function validateEvidence(e, row, batch) {
  assert(['correctness-case-pass', 'performance-case-pass'].includes(e.status), 'complete accepted window required');
  assert.equal(e.error, null, 'failed run');
  assert.equal(e.source.commit, batch.source, 'source mismatch');
  assert.equal(e.source.git_status, '', 'dirty source');
  assert.match(e.source.commit, /^[a-f0-9]{40}$/);
  assert.match(e.source.tree, /^[a-f0-9]{40}$/);
  assert.equal(e.source.workers, 1);
  for (const key of ['cargo_lock', 'workspace_manifest', 'harness_manifest', 'adapter_manifest', 'spatial_manifest', 'binary']) {
    assert.match(e.source[key]?.sha256 ?? '', /^[a-f0-9]{64}$/, `missing ${key}`);
    assert(e.source[key].bytes > 0, `empty ${key}`);
  }
  for (const key of ['hardware_role', 'power_role', 'os', 'architecture', 'rustc', 'cargo']) {
    assert(typeof e.source[key] === 'string' && e.source[key].trim(), `missing ${key}`);
  }
  assert.equal(e.source.allocation, row.kind === 'allocation', 'wrong allocation build');
  assert.equal(e.source.pose_profiling, row.kind === 'profile', 'wrong profile build');
  assert.equal(e.source.binary.sha256, batch.builds.find(b => b.name === row.kind)?.sha256, 'binary mismatch');
  assert.equal(e.mode, 'adapter');
  assert.equal(e.prefix_ticks, null, 'prefix forbidden');
  assert.equal(e.wall_limit_ms, null, 'bounded probe forbidden');
  assert.equal(e.window.purpose, e.status === 'performance-case-pass' ? 'performance' : 'correctness');
  assert.equal(e.completed_ticks, e.window.warm_up_ticks + e.window.observation_ticks, 'partial window');
  assert.equal(e.target_ticks, e.completed_ticks);
  assert.equal(e.committed_world_tick, e.completed_ticks);
  assert.equal(e.stop_reason, 'tick-limit');
  assert.equal(e.execution_id, row.execution_id);
  assert.equal(e.presentation_mode.mode, row.configuration === 'original' ? 'FullValidation' : row.configuration.endsWith('-full') ? 'FullValidationSelected' : 'SelectedPresentation');
  if (row.configuration !== 'original') {
    assert.equal(e.presentation_mode.selection.stride, row.configuration.startsWith('stable-') ? 0 : 137);
    assert.equal(e.presentation_mode.selection.offset, 53);
    assert.equal(e.presentation_mode.selection.reverse, true);
  }
}

export function validateMatrix(rows) {
  const expected = [];
  for (const kind of ['wall', 'allocation', 'profile']) {
    for (let round = 1; round <= (kind === 'wall' ? 3 : 1); round++) {
      for (const configuration of configurations) {
        if (kind !== 'wall' && configuration === 'original') continue;
        expected.push(`${kind}-${configuration}-${round}`);
      }
    }
  }
  equal(rows.map(r => r.name).sort(), expected.sort(), 'missing or duplicate process rows');
  assert.equal(new Set(rows.map(r => r.execution_id)).size, rows.length, 'reused execution');
  for (const row of rows) equal(row.name, `${row.kind}-${row.configuration}-${row.round}`, 'row identity');
}

function compareTraffic(left, right) {
  const traffic = record => { const {N_presented, ...rest} = record; return rest; };
  equal(traffic(left.runtime), traffic(right.runtime), 'traffic differs');
  equal(left.commands, right.commands, 'commands differ');
  equal(left.events, right.events, 'events differ');
}
export function compareFrame(left, right) {
  compareTraffic(left, right);
  for (const field of ['tick', 'individual', 'active', 'presentable', 'applied', 'applied_digest']) {
    equal(left.presentation[field], right.presentation[field], `presentation ${field} differs`);
  }
}
function summary(values) {
  assert(values.length, 'no measured samples');
  values.sort((a, b) => a - b);
  const percentile = p => values[Math.max(0, Math.ceil(values.length * p / 100) - 1)];
  return {samples: values.length, min: values[0], p50: percentile(50), p95: percentile(95), p99: percentile(99), max: values.at(-1)};
}
const metrics = ['selection_ns', 'pose_ns', 'conversion_ns', 'selection_mapping_ns', 'apply_ns', 'presentation_ns', 'validation_ns'];

export async function analyze(directory) {
  const batch = await json(join(directory, 'batch.json'));
  assert.equal(batch.complete, true, 'batch unfinished');
  assert(batch.remote_evidence.startsWith(`${batch.source}\t`), 'unreachable source evidence');
  validateMatrix(batch.rows);
  const reports = new Map();
  let common;
  for (const row of batch.rows) {
    const run = join(directory, row.name);
    const bytes = await readFile(join(run, 'evidence.json'));
    equal(digest(bytes), row.evidence_sha256, 'evidence digest');
    const e = JSON.parse(bytes);
    validateEvidence(e, row, batch);
    const provenance = {
      commit: e.source.commit, tree: e.source.tree, lock: e.source.cargo_lock,
      workspace: e.source.workspace_manifest, adapter: e.source.adapter_manifest,
      harness: e.source.harness_manifest, spatial: e.source.spatial_manifest,
      rustc: e.source.rustc, cargo: e.source.cargo, hardware: e.source.hardware_role,
      power: e.source.power_role, os: e.source.os, architecture: e.source.architecture,
      window: e.window, plan: e.plan_digest, artifacts: e.artifact_files,
      manifest: e.manifest_sha256, case: e.case, scale: e.scale,
      checkpoints: e.checkpoints,
    };
    if (!common) common = provenance;
    equal(provenance, common, 'paired provenance or complete checkpoints differ');
    for (const [name, expected] of Object.entries(e.files)) {
      equal(await hashFile(join(run, name)), expected, `file digest: ${name}`);
    }
    equal(e.files['resolved-plan.toml'].sha256, e.plan_digest, 'plan digest mismatch');
    const plan = await readFile(join(run, 'resolved-plan.toml'), 'utf8');
    const cycle = Number(plan.match(/^cycle_ticks = (\d+)$/m)?.[1]);
    assert(Number.isSafeInteger(cycle) && cycle > 0, 'plan cycle missing');
    equal(e.window, e.window.purpose === 'correctness'
      ? {purpose: 'correctness', warm_up_ticks: cycle, observation_ticks: 2 * cycle}
      : {purpose: 'performance', warm_up_ticks: Math.max(4 * cycle, 512), observation_ticks: Math.max(8 * cycle, 4096)}, 'unaccepted window');
    const timings = Object.fromEntries(metrics.map(key => [key, []]));
    const counts = {requested: [], extracted: [], applied: [], presentable: []};
    const allocation = {allocations: 0, reallocations: 0, bytes_allocated: 0, bytes_reallocated: 0};
    const lifecycle = {created: 0, reused: 0, hidden: 0, shown: 0, retired_bindings: 0};
    let tick = 0;
    let storage;
    for await (const frame of frames(join(run, 'frames.jsonl'))) {
      const p = frame.presentation;
      assert.equal(frame.runtime.tick, ++tick, 'missing/duplicate tick');
      assert.equal(p.tick, tick);
      equal(p.mode, e.presentation_mode);
      assert.equal(p.extracted, p.n_presented);
      assert.equal(p.individual, frame.runtime.N_individual);
      assert.equal(p.active, frame.runtime.N_active);
      assert.equal(p.n_presented, frame.runtime.N_presented);
      if (e.presentation_mode.mode === 'SelectedPresentation') {
        assert.equal(p.requested, Math.floor(p.individual * p.mode.selection.percent / 100));
        assert.equal(p.applied, p.extracted);
        assert(p.extracted <= p.requested && p.extracted <= p.presentable);
      } else { assert.equal(p.extracted, p.presentable); }
      if (row.kind === 'allocation') assert(p.allocation, 'allocation samples absent');
      else assert.equal(p.allocation, null, 'allocation data in wrong build');
      for (const [len, capacity] of Object.values(p.storage)) assert(len <= capacity, 'invalid capacity');
      if (tick > e.window.warm_up_ticks) {
        for (const metric of metrics) { assert(Number.isFinite(p[metric]) && p[metric] >= 0); timings[metric].push(p[metric]); }
        for (const count of Object.keys(counts)) counts[count].push(p[count]);
        if (p.allocation) for (const key of Object.keys(allocation)) allocation[key] += p.allocation[key];
        for (const key of Object.keys(lifecycle)) lifecycle[key] += p[key];
      }
      storage = p.storage;
    }
    assert.equal(tick, e.completed_ticks, 'truncated frame log');
    const report = {kind: row.kind, configuration: row.configuration, round: row.round,
      timings: Object.fromEntries(metrics.map(key => [key, summary(timings[key])])),
      counts: Object.fromEntries(Object.entries(counts).map(([key, values]) => [key, summary(values)])),
      allocation: row.kind === 'allocation' ? allocation : null, lifecycle, final_storage: storage};
    if (row.kind === 'profile') {
      const groups = {'pose-source': [], 'pose-spatial': [], 'pose-adapter': []};
      for await (const line of createInterface({input: createReadStream(join(directory, `${row.name}.stderr.log`)), crlfDelay: Infinity})) {
        const [name, ...parts] = line.split(' ');
        if (!(name in groups)) continue;
        groups[name].push(Object.fromEntries(parts.map(part => part.split('=')).map(([key, value]) => [key, key === 'mode' ? value : Number(value)])));
      }
      report.profile = {};
      for (const [name, samples] of Object.entries(groups)) {
        assert.equal(samples.length, tick, `incomplete ${name} log`);
        report.profile[name] = {last: samples.at(-1), timings: {}};
        for (const key of Object.keys(samples[0]).filter(key => key.endsWith('_ns'))) {
          report.profile[name].timings[key] = summary(samples.slice(e.window.warm_up_ticks).map(s => s[key]));
        }
      }
    }
    reports.set(row.name, report);
  }
  // 配对流逐 tick 比较交通、命令、事件和最终应用摘要，而非仅比较末帧数量。
  for (const kind of ['wall', 'allocation', 'profile']) {
    for (const pattern of ['stable', 'dynamic']) {
      for (let round = 1; round <= (kind === 'wall' ? 3 : 1); round++) {
        const right = frames(join(directory, `${kind}-${pattern}-selected-${round}`, 'frames.jsonl'))[Symbol.asyncIterator]();
        for await (const left of frames(join(directory, `${kind}-${pattern}-full-${round}`, 'frames.jsonl'))) {
          const next = await right.next(); assert(!next.done, 'selected log ended early'); compareFrame(left, next.value);
        }
        assert((await right.next()).done, 'selected log too long');
      }
    }
  }
  // 所有轮次/配置也逐 tick 对齐原全量交通，不能仅由末态检查点推断中途一致。
  for (const row of batch.rows.filter(r => r.name !== 'wall-original-1')) {
    const right = frames(join(directory, row.name, 'frames.jsonl'))[Symbol.asyncIterator]();
    for await (const left of frames(join(directory, 'wall-original-1', 'frames.jsonl'))) {
      const next = await right.next(); assert(!next.done, 'traffic log ended early'); compareTraffic(left, next.value);
    }
    assert((await right.next()).done, 'traffic log too long');
  }
  const aggregate = {};
  for (const configuration of configurations) {
    const rounds = [1, 2, 3].map(round => reports.get(`wall-${configuration}-${round}`));
    aggregate[configuration] = Object.fromEntries(metrics.map(metric => [metric, {
      samples: rounds.reduce((n, row) => n + row.timings[metric].samples, 0),
      ...Object.fromEntries(['p50', 'p95', 'p99'].map(key => [key, summary(rounds.map(row => row.timings[metric][key])).p50])),
      max: Math.max(...rounds.map(row => row.timings[metric].max)),
    }]));
  }
  const result = {status: 'complete-paired-chain-verified', source: batch.source, provenance: common,
    scope: 'same accepted complete window; warmup excluded; normal wall, allocator and phase profiling separate; no product certification',
    aggregation: 'median of three independent round percentiles; maximum is worst round; samples not pooled',
    aggregate, rows: Object.fromEntries(reports)};
  await writeFile(join(directory, 'analysis.json'), JSON.stringify(result, null, 2) + '\n');
  return result;
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { const result = await analyze(resolve(process.argv[2])); console.log(result.status); }
  catch (error) { console.error(error); process.exitCode = 1; }
}
