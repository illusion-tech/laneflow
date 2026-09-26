import {test} from 'node:test';
import assert from 'node:assert/strict';
import {validateEvidence, validateMatrix, compareFrame} from './analyze.mjs';

const sha = 'a'.repeat(64);
const commit = 'b'.repeat(40);
function evidence() {
  const file = {bytes: 1, sha256: sha};
  const source = {commit, tree: commit, git_status: '', workers: 1, allocation: false, pose_profiling: false,
    binary: file, cargo_lock: file, workspace_manifest: file, harness_manifest: file,
    adapter_manifest: file, spatial_manifest: file, hardware_role: 'cpu', power_role: 'balanced',
    os: 'windows', architecture: 'x86_64', rustc: 'rustc 1.98.0', cargo: 'cargo 1.98.0'};
  return {status: 'correctness-case-pass', error: null, source, mode: 'adapter', prefix_ticks: null,
    wall_limit_ms: null, window: {purpose: 'correctness', warm_up_ticks: 10, observation_ticks: 20},
    completed_ticks: 30, target_ticks: 30, committed_world_tick: 30, stop_reason: 'tick-limit',
    execution_id: 'one', presentation_mode: {mode: 'SelectedPresentation', selection: {percent: 10, offset: 53, stride: 0, reverse: true}}};
}
const row = {kind: 'wall', configuration: 'stable-selected', execution_id: 'one'};
const batch = {source: commit, builds: [{name: 'wall', sha256: sha}]};

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
