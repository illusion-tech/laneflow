//! #960 宿主审计判据；不属于 Runtime API，也不登记宿主检查点。

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub command_cursor: u64,
    pub tick: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    pub position: Position,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Boundary {
    Cutover,
    Restore,
    SemanticConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Failure {
    Execution(String),
    Digest(String),
    RootAuthentication,
    Transport(String),
    MissingPoints,
    MissingTarget,
    Boundary(Boundary),
}

#[derive(Debug)]
pub struct Trace {
    pub points: Vec<Point>,
    pub failure: Option<Failure>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum AuditResult {
    CompleteMatch,
    Diverged {
        last_match: Option<Position>,
        expected: Point,
        actual: Point,
    },
    Incomplete {
        target: Position,
        last_completed: Option<Position>,
        reason: Failure,
    },
}

/// `expected` 包含起点和宿主要求的全部比较点；`target` 独立于传输轨迹。
/// 管理边界由宿主分段；带边界的区间整体未覆盖，不把边界后匹配冒充全程匹配。
pub fn compare(
    expected: &Trace,
    actual: &Trace,
    target: Position,
    boundary: Option<Boundary>,
) -> AuditResult {
    let incomplete = |reason| AuditResult::Incomplete {
        target,
        last_completed: actual.points.last().map(|point| point.position),
        reason,
    };
    if let Some(boundary) = boundary {
        return incomplete(Failure::Boundary(boundary));
    }
    if let Some(reason) = actual.failure.as_ref().or(expected.failure.as_ref()) {
        return incomplete(reason.clone());
    }
    // 完成性先于前缀比较；即使前缀不同，失败或缺点轨迹仍未完成目标审计。
    if expected.points.is_empty() || expected.points.len() != actual.points.len() {
        return incomplete(Failure::MissingPoints);
    }
    if expected.points.last().map(|point| point.position) != Some(target)
        || actual.points.last().map(|point| point.position) != Some(target)
    {
        return incomplete(Failure::MissingTarget);
    }
    let mut last_match = None;
    for (expected, actual) in expected.points.iter().zip(&actual.points) {
        if expected != actual {
            return AuditResult::Diverged {
                last_match,
                expected: *expected,
                actual: *actual,
            };
        }
        last_match = Some(expected.position);
    }
    AuditResult::CompleteMatch
}
