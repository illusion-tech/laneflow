//! 路权策略阶段的共同诊断与有界计量。

use crate::{CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, SourceLocation};
/// 路权策略在各编译阶段共用的只读数据模型。
pub(crate) mod model;

/// 身份文本的确定性比较：先按长度、再按字节序。
pub(crate) fn compare_identity_text(a: &str, b: &str) -> core::cmp::Ordering {
    (a.len() as u32)
        .to_le_bytes()
        .cmp(&(b.len() as u32).to_le_bytes())
        .then_with(|| a.as_bytes().cmp(b.as_bytes()))
}

/// 规则输入或静态解析不能成立的结构化原因。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PolicyViolation {
    /// 证据/间隙参数成员键、stream/gate 规则键或规则内 `evidence_keys`/
    /// `gap_profile_key` 引用键违反外部 token 文本规则、含 `::` 分隔符，或
    /// owner 限定引用携带超过 3 个 owner 键（`add_right_of_way_policy_set`；
    /// LFRE 的同类键在预检即以 `InvalidText` 拒绝）。
    InvalidKey,
    /// 证据 locator 或间隙参数 `parameter_version` 为空字符串
    /// （`add_right_of_way_policy_set`）。
    EmptyValue,
    /// 法规身份的 jurisdiction/version/source 字符数不在 `1..=128`
    /// （`add_right_of_way_policy_set` 准入可达；LFRE 来源的非法法规身份在
    /// `add_road_editing_module` 预检即以 `InvalidCombination` 拒绝，compile 侧
    /// 重验同 `DuplicateMember` 为防御性）。
    InvalidRegulation,
    /// 策略任一来源位置（主或参与）的 `sourceDocumentKey` 与所属模块登记键不一致
    /// （`add_right_of_way_policy_set`）。
    SourceDocument,
    /// 同一策略集内证据、间隙参数、stream 规则或 gate 规则的成员键重复
    /// （`add_right_of_way_policy_set` 与 `add_road_editing_module` 准入可达；
    /// `Compiler::compile` 的重复校验为防御性——经公开 API 构建的编译单元
    /// 不会首先在此触发）。
    DuplicateMember,
    /// 让行目标、参与者类别或证据引用在同一规则内重复（两个准入入口可达；
    /// compile 侧重验同 `DuplicateMember` 为防御性）。
    DuplicateReference,
    /// stream/gate 规则显式声明的参与者类别列表为空（`add_right_of_way_policy_set`
    /// 准入可达；LFRE 的空类别向量在预检即以 `EmptyCollection` 拒绝，compile 侧
    /// 重验同 `DuplicateMember` 为防御性）。
    EmptyClasses,
    /// 规则引用的证据键或间隙参数键不存在于本策略集的对应成员中（两个准入
    /// 入口可达；compile 侧重验同 `DuplicateMember` 为防御性）。
    MissingLocalReference,
    /// 规则没有证据成员，且所属法规身份也没有来源说明（两个准入入口可达；
    /// compile 侧重验同 `DuplicateMember` 为防御性）。
    MissingEvidence,
    /// stream 规则的让行目标与间隙参数必须同有或同无：声明了让行目标但缺间隙
    /// 参数，或未声明目标却携带间隙参数（两者皆无为合法；
    /// `add_right_of_way_policy_set` 准入可达，LFRE 的不一致在预检即以
    /// `InvalidCombination` 拒绝，compile 侧重验同 `DuplicateMember` 为防御性）。
    GapBinding,
    /// 某参与类别被车辆 profile 使用并在机动门或参与者流上可准入，但该策略集
    /// 内没有任何可适用规则（多策略集时每个集合都须独立覆盖）；选择单元只为
    /// profile 使用的类别建立，未被使用的类别不触发（`Compiler::compile` 的
    /// MIR 策略校验）。
    MissingRule,
    /// 同一策略集内，同一门上两条规则对同一参与类别的特异性并列（门规则无
    /// 优先级轴），或同一流上两条规则的特异性与优先级均并列，无法唯一选择
    /// 规则；流规则仅特异性并列但优先级不同时按优先级确定性选择，跨策略集的
    /// 并列不触发（`Compiler::compile` 的 MIR 策略校验）。
    AmbiguousRule,
    /// 门规则的解释不是 Uncontrolled 但所引用机动门未绑定信号组、Uncontrolled
    /// 解释与既有信号组绑定不一致，或未绑定门规则携带 OnRed 禁止
    /// （`Compiler::compile` 的 MIR 策略校验）。
    SignalBinding,
    /// 门规则使用右转灯组解释，但对应 movement 的转向不是右转
    /// （`Compiler::compile` 的 MIR 策略校验）。
    RightTurnRequired,
    /// 同一机动门被门规则绑定为不同灯组类型；灯组累计跨策略集构建，冲突可跨
    /// 策略集出现（`Compiler::compile` 的 MIR 策略校验）。
    LampTypeConflict,
    /// stream 规则的让行目标包含自身（`Compiler::compile` 的 MIR 策略校验）。
    SelfYield,
    /// 让行目标流与本流不存在任何共同冲突区——两流各自的冲突通行段不落入同一
    /// 冲突区即判不相交（`Compiler::compile` 的 MIR 策略校验）。
    DisjointYield,
    /// 被让行目标流可按优先级不高于本规则的规则准入，让行关系不闭合
    /// （`Compiler::compile` 的 MIR 策略校验）。
    YieldPriority,
    /// 策略集的法规 jurisdiction/version 与编译单元准入规则确立的唯一法规身份
    /// 不一致（`Compiler::compile` 的 MIR 策略校验）。
    RegulationMismatch,
    /// 同一冲突区的保护放行门信号不相容：保护门分属不同信号控制器（任一信号
    /// 组无绿灯相位时跳过该组比较），或同一信号相位会同时放行同区两组保护门
    /// （`Compiler::compile` 的 MIR 策略校验）。
    ProtectedConflict,
}

/// 构造单条结构化策略违规诊断。
pub(crate) fn error(
    key: &str,
    member: Option<&str>,
    violation: PolicyViolation,
    source: &SourceLocation,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_policy(
        key,
        member,
        violation,
        source.clone(),
    ))
}

/// 核对策略阶段的暂存与存续字节是否超出所选编译上限。
pub(crate) fn check_budget(
    limits: &CompileLimits,
    scratch: u64,
    live: u64,
) -> Result<(), DiagnosticBundle> {
    for (dimension, actual) in [
        (CompileLimitDimension::StageScratchBytes, scratch),
        (CompileLimitDimension::CompilerControlledLiveBytes, live),
    ] {
        let limit = limits.value(dimension);
        if actual > limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(dimension, limit, actual),
            ));
        }
    }
    Ok(())
}

/// 校验单个路权策略声明的模块内局部约束：成员唯一、引用齐备、证据绑定完整。
pub(crate) fn validate_local_declaration(
    policy: &crate::declaration::RightOfWayPolicySetDeclaration,
) -> Result<(), DiagnosticBundle> {
    use PolicyViolation as V;
    let key = policy.header.stable_key.as_ref();
    if policy.regulation.validate().is_err() {
        return Err(error(key, None, V::InvalidRegulation, &policy.header.span));
    }
    macro_rules! unique_members {
        ($members:expr) => {
            for pair in $members.windows(2) {
                if pair[0].key >= pair[1].key {
                    return Err(error(
                        key,
                        Some(&pair[1].key),
                        V::DuplicateMember,
                        &pair[1].source.primary,
                    ));
                }
            }
        };
    }
    unique_members!(policy.evidence);
    unique_members!(policy.gap_profiles);
    unique_members!(policy.stream_rules);
    unique_members!(policy.gate_rules);
    let evidence = |rule: &str,
                    values: &[std::sync::Arc<str>],
                    location: &SourceLocation|
     -> Result<(), DiagnosticBundle> {
        let fail = |violation| error(key, Some(rule), violation, location);
        if values.is_empty() && policy.regulation.source.is_none() {
            return Err(fail(V::MissingEvidence));
        }
        if values.windows(2).any(|v| v[0] >= v[1]) {
            return Err(fail(V::DuplicateReference));
        }
        for value in values {
            if policy
                .evidence
                .binary_search_by(|entry| entry.key.cmp(value))
                .is_err()
            {
                return Err(fail(V::MissingLocalReference));
            }
        }
        Ok(())
    };
    for rule in &policy.stream_rules {
        let fail = |violation| error(key, Some(&rule.key), violation, &rule.source.primary);
        if rule.classes.as_ref().is_some_and(|v| v.is_empty()) {
            return Err(fail(V::EmptyClasses));
        }
        if rule.yield_to.is_empty() != rule.gap.is_none() {
            return Err(fail(V::GapBinding));
        }
        if let Some(gap) = &rule.gap
            && policy
                .gap_profiles
                .binary_search_by(|v| v.key.cmp(gap))
                .is_err()
        {
            return Err(fail(V::MissingLocalReference));
        }
        if duplicate_references(&rule.yield_to)
            || rule
                .classes
                .as_ref()
                .is_some_and(|v| duplicate_references(v))
        {
            return Err(fail(V::DuplicateReference));
        }
        evidence(&rule.key, &rule.evidence, &rule.source.primary)?;
    }
    for rule in &policy.gate_rules {
        let fail = |violation| error(key, Some(&rule.key), violation, &rule.source.primary);
        if rule.classes.as_ref().is_some_and(|v| v.is_empty()) {
            return Err(fail(V::EmptyClasses));
        }
        if rule
            .classes
            .as_ref()
            .is_some_and(|v| duplicate_references(v))
        {
            return Err(fail(V::DuplicateReference));
        }
        evidence(&rule.key, &rule.evidence, &rule.source.primary)?;
    }
    Ok(())
}

/// 按模块命名空间与目标地址把有类型引用排成规范顺序。
pub(crate) fn sort_references<K: laneflow_static_contract::EntityKindMarker>(
    values: &mut [crate::declaration::OwnedEntityReference<K>],
) {
    values.sort_unstable_by(|a, b| {
        a.module_namespace
            .cmp(&b.module_namespace)
            .then(a.target_address.cmp(&b.target_address))
    });
}
fn duplicate_references<K: laneflow_static_contract::EntityKindMarker>(
    values: &[crate::declaration::OwnedEntityReference<K>],
) -> bool {
    values.windows(2).any(|v| {
        v[0].module_namespace == v[1].module_namespace && v[0].target_address == v[1].target_address
    })
}
