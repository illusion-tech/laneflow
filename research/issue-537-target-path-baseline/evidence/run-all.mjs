// #537 一次性执行编排：批 1 计时件 → 批 2 规模档 → 批 3 生成器两档。
// 运行前提：全部二进制已预编译（PREP_BUILD_OK）；仅在用户授予的窗口内启动。
// 用法：node run-all.mjs <evidence-root>/issue-537/6cf90563
import { spawnSync } from "node:child_process";
import { mkdirSync, appendFileSync, existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2];
if (!root || !existsSync(root)) {
  console.error("usage: node run-all.mjs <evidence-dir>");
  process.exit(2);
}
const log = (line) => appendFileSync(join(root, "run-all.log"), `${new Date().toISOString()} ${line}\n`);
const powerOk = () =>
  spawnSync("powershell", ["-NoProfile", "-Command", "(Get-CimInstance Win32_Battery).BatteryStatus"], { encoding: "utf8" })
    .stdout.trim() === "2";
const heavyProcs = () =>
  spawnSync("tasklist", ["/FO", "CSV"], { encoding: "utf8" })
    .stdout.split("\n")
    .filter((l) => /cargo|rustc|clippy/i.test(l)).length;

const RUNS = [
  // 批 1 计时件（未插桩墙钟，#[ignore] 手动）
  { name: "b1-shared-network-wall", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "shared_network_resource_evidence", "--", "--ignored", "wall_clock_build_identity_and_kernel", "--nocapture", "--test-threads=1"] },
  { name: "b1-snapshot-wall", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "snapshot_wall_clock_evidence", "--", "--ignored", "--nocapture"] },
  { name: "b1-cutover-wall", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "cutover_wall_clock_evidence", "--", "--ignored", "--nocapture"] },
  { name: "b1-cutover-cross-wall", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "cutover_cross_wall_clock_evidence", "--", "--ignored", "--nocapture"] },
  { name: "b1-instrumented-calibrate", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "shared_network_instrumented_wall_clock_evidence", "--", "--ignored", "--nocapture"] },
  // 批 2 规模档（#[ignore] 手动）
  { name: "b2-cutover-scale-wall", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "cutover_scale_evidence", "--", "--ignored", "--nocapture", "--test-threads=1"] },
  { name: "b2-cutover-scale-alloc", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "cutover_scale_allocation", "--", "--ignored", "--nocapture", "--test-threads=1"] },
  { name: "b2-runtime-profile-wall", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "runtime_profile_evidence", "runtime_profile_wall_clock", "--", "--ignored", "--nocapture", "--test-threads=1"] },
  { name: "b2-runtime-profile-alloc", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--test", "runtime_profile_allocation", "--", "--ignored", "--nocapture", "--test-threads=1"] },
  { name: "b2-runtime-profile-stages", cmd: ["cargo", "+1.98.0", "test", "--release", "--locked", "-p", "laneflow-runtime", "--lib", "runtime_profile_stages", "--", "--ignored", "--nocapture", "--test-threads=1"] },
  // 批 3 生成器两档（插桩口径，含冻结清单落盘）
  { name: "b3-generator-10k", cmd: ["target/release/laneflow-urban-generator", "--config", "examples/config/cn-urban.toml", "--scale", "10k", "--output", join(root, "cn-urban-10k")] },
  { name: "b3-generator-100k", cmd: ["target/release/laneflow-urban-generator", "--config", "examples/config/cn-urban.toml", "--scale", "100k", "--output", join(root, "cn-urban-100k")] },
];

log(`=== run-all start; runs=${RUNS.length} ===`);
for (const run of RUNS) {
  if (!powerOk()) { log(`ABORT before ${run.name}: BatteryStatus != 2（电源门禁失败）`); console.error(`ABORT ${run.name}: power gate`); process.exit(1); }
  const heavy = heavyProcs();
  if (heavy > 0) { log(`ABORT before ${run.name}: 检测到 ${heavy} 个并发编译/测试进程`); console.error(`ABORT ${run.name}: heavy procs=${heavy}`); process.exit(1); }
  const out = join(root, `${run.name}.txt`);
  log(`RUN ${run.name} start`);
  const t0 = Date.now();
  const r = spawnSync(run.cmd[0], run.cmd.slice(1), { cwd: process.cwd(), encoding: "utf8", maxBuffer: 1 << 28 });
  const ms = Date.now() - t0;
  appendFileSync(out, `$ ${run.cmd.join(" ")}\n${r.stdout ?? ""}\n${r.stderr ?? ""}\nexit=${r.status}\n`);
  log(`RUN ${run.name} exit=${r.status} elapsed_ms=${ms}`);
  console.log(`${run.name}: exit=${r.status} ${(ms / 1000).toFixed(1)}s`);
  if (r.status !== 0) { log(`STOP：${run.name} 非零退出，按停止条件停止扩大测量`); process.exit(1); }
}
log(`=== run-all done ===`);
console.log("ALL OK");
