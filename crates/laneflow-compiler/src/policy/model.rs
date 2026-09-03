//! 只读声明语义在各编译阶段仅改写有类型引用。
use crate::{GateInterpretation, GateProhibition, RegulationIdentity};
use laneflow_static_contract::RightOfWayPolicySetId;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Evidence {
    pub key: Arc<str>,
    pub locator: Arc<str>,
    pub description: Option<Arc<str>>,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Gap {
    pub key: Arc<str>,
    pub parameter_version: Arc<str>,
    pub lead_ms: u64,
    pub lag_ms: u64,
    pub clearance_ms: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StreamRule<S, C> {
    pub key: Arc<str>,
    pub stream: S,
    pub classes: Option<Box<[C]>>,
    pub priority: i32,
    pub yield_to: Box<[S]>,
    pub gap: Option<Arc<str>>,
    pub evidence: Box<[Arc<str>]>,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GateRule<G, C> {
    pub key: Arc<str>,
    pub gate: G,
    pub classes: Option<Box<[C]>>,
    pub interpretation: GateInterpretation,
    pub prohibition: GateProhibition,
    pub evidence: Box<[Arc<str>]>,
}
#[derive(Debug, PartialEq)]
pub(crate) struct PolicySet<G, S, C> {
    pub id: RightOfWayPolicySetId,
    pub namespace: Arc<str>,
    pub key: Arc<str>,
    pub regulation: RegulationIdentity<Arc<str>>,
    pub evidence: Box<[Evidence]>,
    pub gaps: Box<[Gap]>,
    pub streams: Box<[StreamRule<S, C>]>,
    pub gates: Box<[GateRule<G, C>]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PolicyOrigin {
    pub module: u32,
    pub declaration: u32,
}
#[derive(Debug, PartialEq)]
pub(crate) struct PolicyRecord<G, S, C> {
    pub value: PolicySet<G, S, C>,
    pub origin: PolicyOrigin,
}

impl<G: Copy, S: Copy, C: Copy> PolicySet<G, S, C> {
    pub(crate) fn logical_bytes(&self) -> u64 {
        let text = |s: &str| 4_u64.saturating_add(s.len() as u64);
        let strings = |v: &[Arc<str>]| v.iter().fold(4_u64, |n, s| n.saturating_add(text(s)));
        let classes = |v: &Option<Box<[C]>>| {
            1_u64.saturating_add(v.as_ref().map_or(0, |v| {
                4_u64.saturating_add((v.len() as u64).saturating_mul(4))
            }))
        };
        let mut n = 4_u64
            + 16
            + 8
            + 16
            + text(&self.regulation.jurisdiction)
            + text(&self.regulation.version)
            + 1
            + self.regulation.source.as_ref().map_or(0, |s| text(s));
        for v in &self.evidence {
            n = n
                .saturating_add(text(&v.key))
                .saturating_add(text(&v.locator))
                .saturating_add(1 + v.description.as_ref().map_or(0, |s| text(s)));
        }
        for v in &self.gaps {
            n = n
                .saturating_add(text(&v.key))
                .saturating_add(text(&v.parameter_version))
                .saturating_add(24);
        }
        for v in &self.streams {
            n = n
                .saturating_add(text(&v.key))
                .saturating_add(classes(&v.classes))
                .saturating_add(13)
                .saturating_add((v.yield_to.len() as u64).saturating_mul(4))
                .saturating_add(v.gap.as_ref().map_or(0, |s| text(s)))
                .saturating_add(strings(&v.evidence));
        }
        for v in &self.gates {
            n = n
                .saturating_add(text(&v.key))
                .saturating_add(classes(&v.classes))
                .saturating_add(6)
                .saturating_add(strings(&v.evidence));
        }
        n
    }
    pub(crate) fn text_bytes(&self) -> u64 {
        let mut n = self.namespace.len() as u64 + self.key.len() as u64;
        let mut add = |s: &str| {
            n = n
                .saturating_add(s.len() as u64)
                .saturating_add(2 * size_of::<usize>() as u64)
        };
        add(&self.regulation.jurisdiction);
        add(&self.regulation.version);
        if let Some(s) = &self.regulation.source {
            add(s);
        }
        for v in &self.evidence {
            add(&v.key);
            add(&v.locator);
            if let Some(s) = &v.description {
                add(s);
            }
        }
        for v in &self.gaps {
            add(&v.key);
            add(&v.parameter_version);
        }
        for v in &self.streams {
            add(&v.key);
            if let Some(s) = &v.gap {
                add(s);
            }
            for s in &v.evidence {
                add(s);
            }
        }
        for v in &self.gates {
            add(&v.key);
            for s in &v.evidence {
                add(s);
            }
        }
        n
    }
    pub(crate) fn map<G2, S2, C2>(
        &self,
        gate: impl Fn(G) -> G2,
        stream: impl Fn(S) -> S2,
        class: impl Fn(C) -> C2,
    ) -> PolicySet<G2, S2, C2> {
        PolicySet {
            id: self.id,
            namespace: Arc::clone(&self.namespace),
            key: Arc::clone(&self.key),
            regulation: self.regulation.clone(),
            evidence: self.evidence.clone(),
            gaps: self.gaps.clone(),
            streams: self
                .streams
                .iter()
                .map(|v| StreamRule {
                    key: Arc::clone(&v.key),
                    stream: stream(v.stream),
                    classes: v
                        .classes
                        .as_ref()
                        .map(|v| v.iter().map(|v| class(*v)).collect()),
                    priority: v.priority,
                    yield_to: v.yield_to.iter().map(|v| stream(*v)).collect(),
                    gap: v.gap.clone(),
                    evidence: v.evidence.clone(),
                })
                .collect(),
            gates: self
                .gates
                .iter()
                .map(|v| GateRule {
                    key: Arc::clone(&v.key),
                    gate: gate(v.gate),
                    classes: v
                        .classes
                        .as_ref()
                        .map(|v| v.iter().map(|v| class(*v)).collect()),
                    interpretation: v.interpretation,
                    prohibition: v.prohibition,
                    evidence: v.evidence.clone(),
                })
                .collect(),
        }
    }
    pub(crate) fn records(&self) -> u64 {
        let mut count = 1_u64
            .saturating_add(self.evidence.len() as u64)
            .saturating_add(self.gaps.len() as u64)
            .saturating_add(self.streams.len() as u64)
            .saturating_add(self.gates.len() as u64);
        for v in &self.streams {
            count = count
                .saturating_add(v.classes.as_ref().map_or(0, |v| v.len()) as u64)
                .saturating_add(v.yield_to.len() as u64)
                .saturating_add(v.evidence.len() as u64);
        }
        for v in &self.gates {
            count = count
                .saturating_add(v.classes.as_ref().map_or(0, |v| v.len()) as u64)
                .saturating_add(v.evidence.len() as u64);
        }
        count
    }
    pub(crate) fn owned_bytes(&self) -> u64 {
        let mut count = size_of::<Self>() as u64;
        count = count
            .saturating_add(
                (self.evidence.len() as u64).saturating_mul(size_of::<Evidence>() as u64),
            )
            .saturating_add((self.gaps.len() as u64).saturating_mul(size_of::<Gap>() as u64))
            .saturating_add(
                (self.streams.len() as u64).saturating_mul(size_of::<StreamRule<S, C>>() as u64),
            )
            .saturating_add(
                (self.gates.len() as u64).saturating_mul(size_of::<GateRule<G, C>>() as u64),
            );
        for v in &self.streams {
            count = count
                .saturating_add(
                    (v.classes.as_ref().map_or(0, |v| v.len()) as u64)
                        .saturating_mul(size_of::<C>() as u64),
                )
                .saturating_add((v.yield_to.len() as u64).saturating_mul(size_of::<S>() as u64))
                .saturating_add(
                    (v.evidence.len() as u64).saturating_mul(size_of::<Arc<str>>() as u64),
                );
        }
        for v in &self.gates {
            count = count
                .saturating_add(
                    (v.classes.as_ref().map_or(0, |v| v.len()) as u64)
                        .saturating_mul(size_of::<C>() as u64),
                )
                .saturating_add(
                    (v.evidence.len() as u64).saturating_mul(size_of::<Arc<str>>() as u64),
                );
        }
        count
    }
}
