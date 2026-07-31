#!/usr/bin/env python3
"""从 #308 正式执行检查点独立重算首轮 R0 性能预算。

本脚本只验证会影响预算数据的事实，不实现制品防篡改或生产级证据封套。
"""

from __future__ import annotations

import argparse
import json
from fractions import Fraction
from pathlib import Path
from typing import Any


CHECKPOINT_SCHEMA = "laneflow.compiler-calibration-formal-execution-checkpoint"
REPORT_SCHEMA = "laneflow.compiler-calibration-r0-budget-report"
BATCHES = range(2)
ROUNDS = range(5)
MODES = ("timing", "attribution")
STRATA = (
    ("wall-time-ns", "cold-instance", "timing"),
    ("wall-time-ns", "stable-capacity-reuse", "timing"),
    ("peak-live-requested-bytes", "cold-instance", "attribution"),
    ("peak-live-requested-bytes", "stable-capacity-reuse", "attribution"),
)


class InvalidEvidence(ValueError):
    """原始检查点不能形成预算时使用的稳定失败类型。"""


def require(condition: bool, detail: str) -> None:
    if not condition:
        raise InvalidEvidence(detail)


def positive_int(value: Any, detail: str) -> int:
    require(type(value) is int and value > 0, detail)
    return value


def median_and_mad(values: list[int]) -> tuple[int, int]:
    require(values and len(values) % 2 == 1, "中位数输入必须是非空奇数项")
    require(all(type(value) is int and value > 0 for value in values), "指标必须是正整数")
    ordered = sorted(values)
    median = ordered[len(ordered) // 2]
    deviations = sorted(abs(value - median) for value in values)
    return median, deviations[len(deviations) // 2]


def ratio_median(values: list[Fraction]) -> Fraction:
    require(len(values) == 5, "相邻级别必须形成五个同轮比值")
    return sorted(values)[2]


def candidate_knee(metric: str, ratios: list[Fraction]) -> bool:
    median = ratio_median(ratios)
    if metric == "wall-time-ns":
        return sum(ratio >= Fraction(11, 10) for ratio in ratios) >= 4 and median >= Fraction(
            6, 5
        )
    if metric == "peak-live-requested-bytes":
        return all(ratio >= Fraction(21, 20) for ratio in ratios) and median >= Fraction(
            11, 10
        )
    raise InvalidEvidence(f"未知拐点指标：{metric}")


def validate_sample(sample: dict[str, Any], mode: str, ordinal: int, digest: str) -> int:
    require(sample.get("sampleOrdinal") == ordinal, "样本序号不连续")
    require(sample.get("semanticDigestSha256") == digest, "同一正式级别语义摘要不一致")
    peak = positive_int(
        sample.get("guardPeakLiveRequestedBytes"), "缺少正数 peak-live-requested-bytes"
    )
    if mode == "timing":
        positive_int(sample.get("wallTimeNs"), "timing 样本缺少正数墙钟")
        require(sample.get("attributionWallTimeNsDiagnostic") is None, "timing 混入归因墙钟")
        require(sample.get("allocation") is None, "timing 混入逐分配记账")
    else:
        require(sample.get("wallTimeNs") is None, "attribution 墙钟不得进入时延指标")
        positive_int(
            sample.get("attributionWallTimeNsDiagnostic"), "attribution 缺少诊断墙钟"
        )
        require(isinstance(sample.get("allocation"), dict), "attribution 缺少分配观察")
    return peak


def validate_child(
    child: dict[str, Any],
    mode: str,
    workload_id: str,
    graph_profile: str,
    n: int,
    expected_digest: str,
) -> tuple[int, list[int]]:
    require(child.get("outcome") == "success", "正式子进程未成功")
    require(child.get("binaryMode") == mode, "正式子进程模式错误")
    require(child.get("workloadId") == workload_id, "正式子进程工作负载错误")
    require(child.get("graphProfile") == graph_profile, "正式子进程模块图错误")
    require(child.get("n") == n, "正式子进程规模错误")
    require(child.get("warmupCount") == 3, "正式子进程必须执行三次预热")
    require(
        child.get("allocationInstrumentationEnabled") == (mode == "attribution"),
        "正式子进程分配插桩状态与模式不一致",
    )
    require(child.get("controlledAllocationGuard") is None, "成功子进程仍携带停止护栏")
    cold = child.get("coldInstance")
    reuse = child.get("stableCapacityReuse")
    require(isinstance(cold, dict), "缺少冷实例样本")
    require(isinstance(reuse, list) and len(reuse) == 7, "复用样本必须恰好七项")
    require(child.get("retainedCapacityBytes") is not None, "缺少保留容量观察")

    cold_digest = cold.get("semanticDigestSha256")
    require(cold_digest == expected_digest, "正式子进程与预检/预言机摘要不一致")
    cold_peak = validate_sample(cold, mode, 0, expected_digest)
    reuse_peaks = [
        validate_sample(sample, mode, ordinal, expected_digest)
        for ordinal, sample in enumerate(reuse)
    ]
    if mode == "timing":
        return positive_int(cold.get("wallTimeNs"), "缺少冷实例墙钟"), [
            positive_int(sample.get("wallTimeNs"), "缺少复用墙钟") for sample in reuse
        ]
    return cold_peak, reuse_peaks


def analyze_level(
    ladder: dict[str, Any], level: dict[str, Any]
) -> dict[tuple[int, int, str, str], dict[str, Any]]:
    workload_id = ladder["workloadId"]
    graph_profile = ladder["graphProfile"]
    n = positive_int(level.get("n"), "正式级别 N 无效")
    require(level.get("complete") is True, f"N={n} 未完整完成")
    preflight_run = level.get("attributionPreflight", {})
    oracle_run = level.get("oracle", {})
    require(
        preflight_run.get("status") == "valid" and oracle_run.get("status") == "valid",
        f"N={n} 预检或预言机无效",
    )
    preflight = preflight_run.get("child")
    oracle = oracle_run.get("child")
    require(isinstance(preflight, dict) and isinstance(oracle, dict), f"N={n} 缺少预检或预言机")
    expected_digest = preflight.get("semanticDigestSha256")
    require(
        preflight.get("outcome") == "success"
        and oracle.get("outcome") == "success"
        and oracle.get("completeCountsEqual") is True
        and oracle.get("completeTypedOutputEqual") is True
        and isinstance(expected_digest, str)
        and expected_digest
        and oracle.get("semanticDigestSha256") == expected_digest,
        f"N={n} 预检与独立预言机摘要不一致",
    )

    runs = level.get("formalRuns")
    require(isinstance(runs, list), f"N={n} 缺少正式运行")
    by_identity: dict[tuple[int, int, str], dict[str, Any]] = {}
    for run in runs:
        require(run.get("status") == "valid", f"N={n} 含无效运行")
        require(not run.get("invalidationReasons"), f"N={n} 有效运行仍携带作废原因")
        identity = (run.get("batch"), run.get("round"), run.get("binaryMode"))
        require(
            identity[0] in BATCHES and identity[1] in ROUNDS and identity[2] in MODES,
            f"N={n} 正式运行身份无效",
        )
        require(identity not in by_identity, f"N={n} 正式运行身份重复")
        by_identity[identity] = run
    require(len(by_identity) == 20, f"N={n} 必须恰好包含二十个正式子进程")

    summaries: dict[tuple[int, int, str, str], dict[str, Any]] = {}
    for batch in BATCHES:
        for round_index in ROUNDS:
            timing = by_identity[(batch, round_index, "timing")]["child"]
            attribution = by_identity[(batch, round_index, "attribution")]["child"]
            require(isinstance(timing, dict) and isinstance(attribution, dict), "缺少子进程报告")
            cold_wall, reuse_wall = validate_child(
                timing, "timing", workload_id, graph_profile, n, expected_digest
            )
            cold_peak, reuse_peak = validate_child(
                attribution, "attribution", workload_id, graph_profile, n, expected_digest
            )
            for metric, sample_kind, values in (
                ("wall-time-ns", "cold-instance", [cold_wall]),
                ("wall-time-ns", "stable-capacity-reuse", reuse_wall),
                ("peak-live-requested-bytes", "cold-instance", [cold_peak]),
                ("peak-live-requested-bytes", "stable-capacity-reuse", reuse_peak),
            ):
                median, mad = median_and_mad(values)
                summaries[(batch, round_index, metric, sample_kind)] = {
                    "values": values,
                    "median": median,
                    "medianAbsoluteDeviation": mad,
                }
    return summaries


def batch_summary(
    rounds: dict[tuple[int, int, str, str], dict[str, Any]],
    batch: int,
    metric: str,
    sample_kind: str,
) -> dict[str, Any]:
    round_medians = [rounds[(batch, round_index, metric, sample_kind)]["median"] for round_index in ROUNDS]
    median, mad = median_and_mad(round_medians)
    return {
        "roundMedians": round_medians,
        "median": median,
        "medianAbsoluteDeviation": mad,
    }


def analyze_ladder(ladder: dict[str, Any]) -> dict[str, Any]:
    workload_id = ladder.get("workloadId")
    graph_profile = ladder.get("graphProfile")
    levels = [level for level in ladder.get("levels", []) if level.get("complete") is True]
    require(len(levels) >= 5, f"{workload_id}/{graph_profile} 完整级别少于五项")
    levels.sort(key=lambda level: level["n"])
    for lower, upper in zip(levels, levels[1:]):
        require(lower["n"] * 2 == upper["n"], "正式级别不是严格二倍递增")

    analyzed_levels: dict[int, dict[str, Any]] = {}
    for level in levels:
        n = level["n"]
        rounds = analyze_level(ladder, level)
        batches = {
            (batch, metric, sample_kind): batch_summary(
                rounds, batch, metric, sample_kind
            )
            for batch in BATCHES
            for metric, sample_kind, _mode in STRATA
        }
        analyzed_levels[n] = {
            "n": n,
            "primaryRecordCount": positive_int(
                level.get("primaryRecordCount"), "主归一化记录数无效"
            ),
            "canonicalLirRecordCount": positive_int(
                level.get("canonicalLirRecordCount"), "LIR 归一化记录数无效"
            ),
            "rounds": rounds,
            "batches": batches,
        }

    confirmed_knees: list[int] = []
    ordered = list(analyzed_levels.values())
    for lower, upper in zip(ordered, ordered[1:]):
        for metric, sample_kind, _mode in STRATA:
            normalizer_key = (
                "primaryRecordCount"
                if metric == "wall-time-ns"
                else "canonicalLirRecordCount"
            )
            confirmed = True
            for batch in BATCHES:
                ratios = [
                    Fraction(
                        upper["rounds"][(batch, round_index, metric, sample_kind)]["median"]
                        * lower[normalizer_key],
                        lower["rounds"][(batch, round_index, metric, sample_kind)]["median"]
                        * upper[normalizer_key],
                    )
                    for round_index in ROUNDS
                ]
                confirmed = confirmed and candidate_knee(metric, ratios)
            if confirmed:
                confirmed_knees.append(upper["n"])

    if confirmed_knees:
        stress_n = min(confirmed_knees)
        calibration_n = max(n for n in analyzed_levels if n < stress_n)
        disposition = "confirmed-knee"
    else:
        calibration_n, stress_n = [level["n"] for level in ordered[-2:]]
        disposition = "no-observed-knee"

    embedded = ladder.get("analysis", {}).get("scaleSelection", {})
    require(
        embedded.get("disposition") == disposition
        and embedded.get("calibrationN") == calibration_n
        and embedded.get("stressN") == stress_n,
        f"{workload_id}/{graph_profile} 内嵌规模选择与原始样本重算不一致",
    )
    return {
        "workloadId": workload_id,
        "graphProfile": graph_profile,
        "levels": analyzed_levels,
        "calibrationN": calibration_n,
        "stressN": stress_n,
        "scaleSelectionDisposition": disposition,
    }


def reduced_ratio(numerator: int, denominator: int) -> dict[str, str]:
    ratio = Fraction(numerator, denominator)
    return {"numerator": str(ratio.numerator), "denominator": str(ratio.denominator)}


def summary_id(
    analysis: dict[str, Any], n: int, batch: int, metric: str, sample_kind: str
) -> str:
    return (
        f"recomputed/{analysis['workloadId']}/{analysis['graphProfile']}/n-{n}/"
        f"batch-{batch}/{metric}/{sample_kind}"
    )


def build_report(checkpoint: dict[str, Any]) -> dict[str, Any]:
    require(checkpoint.get("schema") == CHECKPOINT_SCHEMA, "输入不是正式执行检查点")
    clock_quantum = positive_int(
        checkpoint.get("baseScalePilot", {}).get("clockQuantumNs"), "时钟量子无效"
    )
    analyses = []
    skipped_ladders = []
    for ladder in checkpoint.get("formalLadders", []):
        disposition = ladder.get("disposition")
        complete_level_count = sum(
            level.get("complete") is True for level in ladder.get("levels", [])
        )
        if (
            disposition not in ("complete", "guarded-after-minimum-levels")
            or complete_level_count < 5
        ):
            skipped_ladders.append(
                {
                    "workloadId": ladder.get("workloadId"),
                    "graphProfile": ladder.get("graphProfile"),
                    "disposition": disposition,
                    "completeLevelCount": complete_level_count,
                }
            )
            continue
        analyses.append(analyze_ladder(ladder))
    require(analyses, "没有可形成预算的完整正式阶梯")
    unavailable_base_scales = []
    for selection in checkpoint.get("baseScalePilot", {}).get("selections", []):
        if selection.get("b", {}).get("value") is None:
            unavailable_base_scales.append(
                {
                    "workloadId": selection.get("workloadId"),
                    "graphProfile": selection.get("graphProfile"),
                    "reason": selection.get("b", {}).get("reason"),
                }
            )

    envelopes: dict[str, dict[str, Any]] = {}
    for metric in ("wall-time-ns", "peak-live-requested-bytes"):
        maximum: tuple[Fraction, dict[str, Any], int, str] | None = None
        for analysis in analyses:
            for n, level in analysis["levels"].items():
                for candidate_metric, sample_kind, _mode in STRATA:
                    if candidate_metric != metric:
                        continue
                    batch_zero = level["batches"][(0, metric, sample_kind)]["median"]
                    batch_one = level["batches"][(1, metric, sample_kind)]["median"]
                    ratio = Fraction(max(batch_zero, batch_one), min(batch_zero, batch_one))
                    if maximum is None or ratio > maximum[0]:
                        maximum = (ratio, analysis, n, sample_kind)
        require(maximum is not None, f"缺少 {metric} 重复性包络")
        ratio, analysis, n, sample_kind = maximum
        envelopes[metric] = {
            "metric": metric,
            "maximizingBatch0SummaryId": summary_id(
                analysis, n, 0, metric, sample_kind
            ),
            "maximizingBatch1SummaryId": summary_id(
                analysis, n, 1, metric, sample_kind
            ),
            "repeatRatio": {
                "numerator": str(ratio.numerator),
                "denominator": str(ratio.denominator),
            },
        }

    recommendations: list[dict[str, Any]] = []
    for analysis in analyses:
        for scale_role, n in (
            ("calibration", analysis["calibrationN"]),
            ("stress", analysis["stressN"]),
        ):
            level = analysis["levels"][n]
            for metric, sample_kind, mode in STRATA:
                envelope = envelopes[metric]["repeatRatio"]
                envelope_fraction = Fraction(
                    int(envelope["numerator"]), int(envelope["denominator"])
                )
                round_medians = (
                    level["batches"][(0, metric, sample_kind)]["roundMedians"]
                    + level["batches"][(1, metric, sample_kind)]["roundMedians"]
                )
                observed_upper = max(round_medians)
                quantum = clock_quantum if metric == "wall-time-ns" else 1
                scaled = Fraction(observed_upper) * envelope_fraction / quantum
                suggested = ((scaled.numerator + scaled.denominator - 1) // scaled.denominator) * quantum
                recommendations.append(
                    {
                        "workloadId": analysis["workloadId"],
                        "graphProfile": analysis["graphProfile"],
                        "scaleRole": scale_role,
                        "n": n,
                        "metric": metric,
                        "sampleKind": sample_kind,
                        "binaryMode": mode,
                        "batch0SummaryId": summary_id(
                            analysis, n, 0, metric, sample_kind
                        ),
                        "batch1SummaryId": summary_id(
                            analysis, n, 1, metric, sample_kind
                        ),
                        "observedUpper": observed_upper,
                        "repeatRatio": envelope,
                        "roundingQuantum": quantum,
                        "suggestedR0Budget": suggested,
                        "unit": "nanosecond" if metric == "wall-time-ns" else "byte",
                    }
                )

    return {
        "schema": REPORT_SCHEMA,
        "schemaVersion": 1,
        "scope": "R0 research input for #292; not product SLA",
        "coverage": {
            "metrics": ["wall-time-ns", "peak-live-requested-bytes"],
            "sampleKinds": ["cold-instance", "stable-capacity-reuse"],
            "omitted": [
                "retained-capacity-bytes",
                "private-bytes",
                "commit-peak-bytes",
                "candidate-comparison",
                "growth-slope",
            ],
        },
        "verifiedFormalLadderCount": len(analyses),
        "skippedFormalLadders": skipped_ladders,
        "unavailableBaseScales": unavailable_base_scales,
        "clockQuantumNs": clock_quantum,
        "reproducibilityEnvelopes": list(envelopes.values()),
        "recommendations": recommendations,
    }


def markdown_report(report: dict[str, Any]) -> str:
    lines = [
        "# #308 R0 编译器性能预算",
        "",
        "> 这是供 #292 审阅的 R0 研究输入，不是产品 SLA。",
        "",
        f"- 已独立重算正式阶梯：{report['verifiedFormalLadderCount']} 个",
        f"- 跳过不完整/无效正式阶梯：{len(report['skippedFormalLadders'])} 个",
        f"- 未取得 B 的自然身份：{len(report['unavailableBaseScales'])} 个",
        f"- 时钟量子：{report['clockQuantumNs']} ns",
        "- 当前预算只覆盖端到端墙钟与编译器控制峰值请求字节；未覆盖项保留在 JSON 的 "
        "`coverage.omitted` 中",
        "",
        "| 工作负载 | 模块图 | 规模角色 | N | 指标 | 样本 | 观测上界 | 重复性包络 | 建议预算 | 单位 |",
        "| --- | --- | --- | ---: | --- | --- | ---: | ---: | ---: | --- |",
    ]
    for item in report["recommendations"]:
        ratio = item["repeatRatio"]
        lines.append(
            "| {workloadId} | {graphProfile} | {scaleRole} | {n} | {metric} | "
            "{sampleKind} | {observedUpper} | {numerator}/{denominator} | "
            "{suggestedR0Budget} | {unit} |".format(**item, **ratio)
        )
    return "\n".join(lines) + "\n"


def self_test() -> None:
    assert median_and_mad([1, 2, 3, 4, 100]) == (3, 1)
    assert candidate_knee(
        "wall-time-ns",
        [Fraction(11, 10), Fraction(6, 5), Fraction(6, 5), Fraction(6, 5), Fraction(1)],
    )
    assert candidate_knee(
        "peak-live-requested-bytes",
        [Fraction(21, 20), Fraction(21, 20), Fraction(11, 10), Fraction(11, 10), Fraction(11, 10)],
    )
    assert reduced_ratio(12, 8) == {"numerator": "3", "denominator": "2"}
    digest = "00" * 32

    def sample(mode: str, ordinal: int, value: int) -> dict[str, Any]:
        return {
            "sampleOrdinal": ordinal,
            "wallTimeNs": value if mode == "timing" else None,
            "attributionWallTimeNsDiagnostic": value if mode == "attribution" else None,
            "semanticDigestSha256": digest,
            "guardPeakLiveRequestedBytes": value,
            "allocation": {} if mode == "attribution" else None,
        }

    def child(mode: str, n: int, value: int) -> dict[str, Any]:
        return {
            "outcome": "success",
            "binaryMode": mode,
            "workloadId": "LF-COMP-ID-v1",
            "graphProfile": "wide-star-v1",
            "n": n,
            "warmupCount": 3,
            "allocationInstrumentationEnabled": mode == "attribution",
            "coldInstance": sample(mode, 0, value),
            "stableCapacityReuse": [sample(mode, ordinal, value) for ordinal in range(7)],
            "retainedCapacityBytes": {"total": value},
            "controlledAllocationGuard": None,
        }

    levels = []
    for n in (1, 2, 4, 8, 16):
        runs = []
        for batch in BATCHES:
            for round_index in ROUNDS:
                for mode in MODES:
                    value = (100 if mode == "timing" else 200) * n
                    runs.append(
                        {
                            "status": "valid",
                            "batch": batch,
                            "round": round_index,
                            "binaryMode": mode,
                            "child": child(mode, n, value),
                        }
                    )
        levels.append(
            {
                "n": n,
                "complete": True,
                "primaryRecordCount": n,
                "canonicalLirRecordCount": n,
                "attributionPreflight": {
                    "status": "valid",
                    "child": {
                        "outcome": "success",
                        "semanticDigestSha256": digest,
                    },
                },
                "oracle": {
                    "status": "valid",
                    "child": {
                        "outcome": "success",
                        "semanticDigestSha256": digest,
                        "completeCountsEqual": True,
                        "completeTypedOutputEqual": True,
                    },
                },
                "formalRuns": runs,
            }
        )
    synthetic = {
        "schema": CHECKPOINT_SCHEMA,
        "baseScalePilot": {"clockQuantumNs": 10},
        "formalLadders": [
            {
                "workloadId": "LF-COMP-ID-v1",
                "graphProfile": "wide-star-v1",
                "disposition": "complete",
                "levels": levels,
                "analysis": {
                    "scaleSelection": {
                        "disposition": "no-observed-knee",
                        "calibrationN": 8,
                        "stressN": 16,
                    }
                },
            }
        ],
    }
    report = build_report(synthetic)
    assert report["verifiedFormalLadderCount"] == 1
    assert len(report["recommendations"]) == 8
    assert all(
        item["repeatRatio"] == {"numerator": "1", "denominator": "1"}
        for item in report["recommendations"]
    )
    corrupted = json.loads(json.dumps(synthetic))
    corrupted["formalLadders"][0]["levels"][0]["formalRuns"][0]["child"]["coldInstance"][
        "wallTimeNs"
    ] = None
    try:
        build_report(corrupted)
    except InvalidEvidence:
        pass
    else:
        raise AssertionError("指标模式污染必须使重算失败")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkpoint", nargs="?", type=Path)
    parser.add_argument("--json", dest="json_output", type=Path)
    parser.add_argument("--markdown", dest="markdown_output", type=Path)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if arguments.self_test:
        self_test()
        print("预算重算自检通过")
        return
    require(arguments.checkpoint is not None, "缺少正式执行检查点路径")
    require(arguments.json_output is not None, "缺少 --json 输出路径")
    require(arguments.markdown_output is not None, "缺少 --markdown 输出路径")
    checkpoint = json.loads(arguments.checkpoint.read_text(encoding="utf-8"))
    report = build_report(checkpoint)
    arguments.json_output.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    arguments.markdown_output.write_text(markdown_report(report), encoding="utf-8")
    print(
        f"已从 {report['verifiedFormalLadderCount']} 个正式阶梯重算预算："
        f"{arguments.json_output}，{arguments.markdown_output}"
    )


if __name__ == "__main__":
    main()
