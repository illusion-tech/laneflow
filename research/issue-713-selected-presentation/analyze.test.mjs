import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtemp, mkdir, readFile, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve, basename} from 'node:path';
import {analyze, validateEvidence, validateMatrix, compareFrame, validateSelectionPercent} from './analyze.mjs';

const sha = 'a'.repeat(64);
const commit = 'b'.repeat(40);
function evidence() {
  const file = {bytes: 1, sha256: sha};
  const source = {commit, tree: commit, git_status: '', workers: 1, allocation: false, pose_profiling: false,
    binary: file, cargo_lock: file, workspace_manifest: file, harness_manifest: file,
    adapter_manifest: file, spatial_manifest: file, hardware_role: 'cpu', power_role: 'balanced',
    os: 'windows', architecture: 'x86_64', rustc: 'rustc 1.98.0', cargo: 'cargo 1.98.0'};
  return {version: 'urban-cross-layer-evidence-v1', case: 'MIXED-PEAK', scale: '10k',
    plan_digest: sha, manifest_sha256: sha,
    artifact_files: Object.fromEntries(['common.lfre', 'config.toml', 'network.lfca', 'routes.toml', 'topology.lfre'].map(name => [name, file])),
    checkpoints: {0: sha, 10: sha, 30: sha},
    status: 'correctness-case-pass', error: null, source, mode: 'adapter', prefix_ticks: null,
    wall_limit_ms: null, window: {purpose: 'correctness', warm_up_ticks: 10, observation_ticks: 20},
    completed_ticks: 30, target_ticks: 30, committed_world_tick: 30, stop_reason: 'tick-limit',
    execution_id: 'one', presentation_mode: {mode: 'SelectedPresentation', selection: {percent: 10, offset: 53, stride: 0, reverse: true}}};
}
const row = {kind: 'wall', configuration: 'stable-selected', execution_id: 'one'};
const batch = {source: commit, builds: [{name: 'wall', sha256: sha}]};

test('rejects selection percent drift across configurations and rounds', () => {
  const expected = validateSelectionPercent(evidence());
  assert.equal(expected, 10);
  assert.equal(validateSelectionPercent(evidence(), expected), expected);
  for (const percent of [1, 100, -1, 101, 10.5, null]) {
    const changed = evidence(); changed.presentation_mode.selection.percent = percent;
    assert.throws(() => validateSelectionPercent(changed, expected));
  }
});

test('rejects partial, probe, failed, wrong build and source drift', () => {
  validateEvidence(evidence(), row, batch);
  for (const mutate of [
    e => e.completed_ticks--, e => e.committed_world_tick--,
    e => e.prefix_ticks = 12, e => e.wall_limit_ms = 600000,
    e => e.window.purpose = 'probe', e => e.error = 'failure',
    e => e.status = 'bounded-observation-complete', e => e.source.git_status = ' M source.rs',
    e => e.source.allocation = true, e => e.source.pose_profiling = true,
    e => e.source.commit = 'c'.repeat(40), e => e.source.binary.sha256 = 'd'.repeat(64),
    e => delete e.source.cargo_lock, e => e.presentation_mode.selection.stride = 137,
    e => delete e.artifact_files, e => delete e.artifact_files['network.lfca'],
    e => delete e.manifest_sha256, e => delete e.plan_digest, e => delete e.checkpoints,
    e => delete e.checkpoints[30], e => e.scale = 'fixture', e => e.case = 'unknown',
  ]) { const e = evidence(); mutate(e); assert.throws(() => validateEvidence(e, row, batch)); }
});

function matrix() {
  const rows = [];
  for (const kind of ['wall', 'allocation', 'profile']) {
    for (let round = 1; round <= (kind === 'wall' ? 3 : 1); round++) {
      for (const configuration of ['original', 'stable-full', 'stable-selected', 'dynamic-full', 'dynamic-selected']) {
        if (kind !== 'wall' && configuration === 'original') continue;
        const name = `${kind}-${configuration}-${round}`;
        rows.push({name, kind, configuration, round, execution_id: name});
      }
    }
  }
  return rows;
}
test('requires all three independent wall rounds and both diagnostic builds', () => {
  validateMatrix(matrix());
  assert.throws(() => validateMatrix(matrix().slice(1)));
  const duplicate = matrix(); duplicate[0] = duplicate[1];
  assert.throws(() => validateMatrix(duplicate));
  const reused = matrix(); reused[1].execution_id = reused[0].execution_id;
  assert.throws(() => validateMatrix(reused));
});

test('compares actual traffic and Transform digest, while permitting extraction counts to differ', () => {
  const left = {runtime: {tick: 1, N_presented: 100, state_digest: 'same'}, commands: [], events: [],
    presentation: {tick: 1, individual: 100, active: 80, presentable: 90, applied: 9, applied_digest: 'same', extracted: 90}};
  const right = structuredClone(left); right.runtime.N_presented = 9; right.presentation.extracted = 9;
  compareFrame(left, right);
  for (const mutate of [r => r.runtime.state_digest = 'different', r => r.commands.push('changed'),
    r => r.events.push('changed'), r => r.presentation.applied_digest = 'different',
    r => r.presentation.applied++]) {
    const changed = structuredClone(right); mutate(changed); assert.throws(() => compareFrame(left, changed));
  }
});

// 仅构造分析器格式夹具；这些合成数据不作为交通正确性或性能证据。
test('validates the full file pipeline and rejects corrupt, truncated and incomplete evidence', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'laneflow-713-analyzer-'));
  const fileDigest = bytes => ({bytes: Buffer.byteLength(bytes), sha256: createHash('sha256').update(bytes).digest('hex')});
  const writeJson = (path, value) => writeFile(path, JSON.stringify(value));
  const rows = matrix();
  const journal = {complete: true, source: commit, remote_evidence: `${commit}\trefs/heads/test`, rows,
    builds: ['wall', 'allocation', 'profile'].map(name => ({name, sha256: sha}))};
  try {
    for (const row of rows) {
      const run = join(directory, row.name);
      await mkdir(run);
      const e = evidence();
      e.execution_id = row.execution_id;
      e.source.allocation = row.kind === 'allocation';
      e.source.pose_profiling = row.kind === 'profile';
      e.initialized_ms = 1;
      const original = row.configuration === 'original';
      const selected = row.configuration.endsWith('-selected');
      e.presentation_mode = original ? {mode: 'FullValidation'} : {
        mode: selected ? 'SelectedPresentation' : 'FullValidationSelected',
        selection: {percent: 10, offset: 53, stride: row.configuration.startsWith('stable-') ? 0 : 137, reverse: true},
      };
      await writeJson(join(directory, `${row.configuration}.json`), e.presentation_mode);
      const records = Array.from({length: 30}, (_, i) => ({
        runtime: {tick: i + 1, N_individual: 100, N_active: 80, N_presented: selected ? 9 : 90, state_digest: sha},
        commands: [], events: [], presentation: {
          tick: i + 1, individual: 100, active: 80, presentable: 90, requested: selected ? 10 : 100,
          host_selected: original ? 90 : 10, extracted: selected ? 9 : 90, applied: original ? 90 : 9,
          n_presented: selected ? 9 : 90, mode: e.presentation_mode, applied_digest: sha,
          created: 0, reused: 9, shown: 0, hidden: 0, retired_bindings: 0,
          selection_ns: 1, pose_ns: 2, conversion_ns: 3, selection_mapping_ns: 4, apply_ns: 5,
          presentation_ns: 15, validation_ns: 6,
          allocation: row.kind === 'allocation' ? {allocations: 0, reallocations: 0, bytes_allocated: 0, bytes_reallocated: 0} : null,
          storage: Object.fromEntries(['live_order', 'requested', 'requested_index', 'candidates', 'outputs', 'bindings', 'visible', 'previous_visible_scratch', 'extracted_index'].map(key => [key, [10, 16]])),
        },
      }));
      const files = {'resolved-plan.toml': 'cycle_ticks = 10\n', 'frames.jsonl': records.map(r => JSON.stringify(r)).join('\n') + '\n'};
      e.files = {};
      for (const [name, bytes] of Object.entries(files)) { await writeFile(join(run, name), bytes); e.files[name] = fileDigest(bytes); }
      e.plan_digest = e.files['resolved-plan.toml'].sha256;
      const bytes = JSON.stringify(e);
      await writeFile(join(run, 'evidence.json'), bytes);
      row.evidence_sha256 = fileDigest(bytes).sha256;
      if (row.kind === 'profile') {
        const lines = Array.from({length: 30}, () => `pose-source mode=${selected ? 'selected' : 'full'} validation_ns=1 query_ns=2\npose-spatial sample_ns=2 commit_ns=1\npose-adapter commit_ns=1\n`).join('');
        await writeFile(join(directory, `${row.name}.stderr.log`), lines);
      }
    }
    await writeJson(join(directory, 'batch.json'), journal);
    const result = await analyze(directory);
    assert.equal(result.status, 'complete-paired-chain-verified');
    assert.equal(result.aggregate['stable-selected'].presentation_ns.samples, 60);
    assert.equal(result.rows['wall-stable-selected-1'].counts.active.p50, 80);
    assert.equal(result.rows['wall-stable-selected-1'].cold.first_presentation.tick, 1);

    const path = join(directory, 'wall-original-1', 'frames.jsonl');
    const originalFrames = await readFile(path, 'utf8');
    await writeFile(path, originalFrames + 'corrupt');
    await assert.rejects(analyze(directory), /file digest/);
    await writeFile(path, originalFrames);

    // 同时更新外层摘要也不能把截短帧流伪装成完整窗口。
    const evidencePath = join(directory, 'wall-original-1', 'evidence.json');
    const originalEvidence = await readFile(evidencePath, 'utf8');
    const shortened = originalFrames.slice(originalFrames.indexOf('\n') + 1);
    const changed = JSON.parse(originalEvidence);
    changed.files['frames.jsonl'] = fileDigest(shortened);
    const changedEvidence = JSON.stringify(changed);
    const oldDigest = rows[0].evidence_sha256;
    rows[0].evidence_sha256 = fileDigest(changedEvidence).sha256;
    await writeFile(path, shortened);
    await writeFile(evidencePath, changedEvidence);
    await writeJson(join(directory, 'batch.json'), journal);
    await assert.rejects(analyze(directory), /missing\/duplicate tick/);
    await writeFile(path, originalFrames);
    await writeFile(evidencePath, originalEvidence);
    rows[0].evidence_sha256 = oldDigest;
    await writeJson(join(directory, 'batch.json'), journal);

    const profilePath = join(directory, 'profile-stable-full-1.stderr.log');
    await writeFile(profilePath, 'pose-source mode=full validation_ns=1 query_ns=2\n');
    await assert.rejects(analyze(directory), /incomplete pose-source log/);
  } finally {
    assert.equal(resolve(directory, '..'), resolve(tmpdir()));
    assert(basename(directory).startsWith('laneflow-713-analyzer-'));
    await rm(directory, {recursive: true});
  }
});
