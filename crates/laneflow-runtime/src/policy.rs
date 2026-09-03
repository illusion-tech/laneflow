//! 显式世界策略绑定；共享规则只借用共享根，步长派生表由世界独占。
use crate::InstallError;
use laneflow_static_contract::{EntityKind, RightOfWayPolicySetId, RightOfWayPolicySetOrdinal};
use laneflow_static_network::{PolicyView, SharedNetworkRevision};

/// 宿主明确指定的策略稳定身份；业务时间及版本选用规则由宿主拥有。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyPin {
    pub policy: RightOfWayPolicySetId,
}

/// 唯一安装入口的必填选择；没有默认策略或安装后 setter。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldPolicySelection {
    /// 整个共享根没有 Gate、ConflictZone、ParticipantStream 时才合法。
    NotRequired,
    Pinned(PolicyPin),
}

/// 当前世界步长对应的保守间隙；下标与所选策略的 gap_profiles 一致。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DerivedPolicyGap {
    required_lead_ms: u64,
    required_lag_ms: u64,
}
impl DerivedPolicyGap {
    #[must_use]
    pub const fn required_lead_ms(self) -> u64 {
        self.required_lead_ms
    }
    #[must_use]
    pub const fn required_lag_ms(self) -> u64 {
        self.required_lag_ms
    }
}

pub(crate) struct WorldPolicyBinding {
    selection: WorldPolicySelection,
    ordinal: Option<RightOfWayPolicySetOrdinal>,
    gaps: Box<[DerivedPolicyGap]>,
    frontier_proof_horizon_ms: Option<u64>,
}

impl WorldPolicyBinding {
    pub(crate) fn install(
        revision: &SharedNetworkRevision,
        selection: WorldPolicySelection,
        dt: u64,
    ) -> Result<Self, InstallError> {
        let ordinal = match selection {
            WorldPolicySelection::NotRequired => {
                if [
                    EntityKind::ManeuverGate,
                    EntityKind::ConflictZone,
                    EntityKind::ParticipantStream,
                ]
                .iter()
                .any(|kind| revision.traffic().entity_counts().count(*kind) != 0)
                {
                    return Err(InstallError::PolicyRequired);
                }
                None
            }
            WorldPolicySelection::Pinned(pin) => Some(
                revision
                    .identity()
                    .ordinal(pin.policy)
                    .ok_or(InstallError::UnknownPolicy { policy: pin.policy })?,
            ),
        };
        let mut gaps = Vec::new();
        let mut horizon = None;
        if let Some(ordinal) = ordinal {
            let policy = revision
                .policy()
                .policy(ordinal)
                .ok_or(InstallError::UnknownPolicy {
                    policy: match selection {
                        WorldPolicySelection::Pinned(pin) => pin.policy,
                        WorldPolicySelection::NotRequired => unreachable!(),
                    },
                })?;
            policy
                .gap_profiles()
                .len()
                .checked_mul(core::mem::size_of::<DerivedPolicyGap>())
                .filter(|bytes| *bytes <= isize::MAX as usize)
                .ok_or(InstallError::PolicyCapacityOverflow)?;
            gaps.try_reserve_exact(policy.gap_profiles().len())
                .map_err(|_| InstallError::PolicyAllocationFailed)?;
            for (i, gap) in policy.gap_profiles().iter().enumerate() {
                let overflow = InstallError::PolicyGapOverflow {
                    gap_profile_index: u32::try_from(i)
                        .map_err(|_| InstallError::PolicyCapacityOverflow)?,
                };
                let lead = checked_ms(dt, gap.minimum_lead_ms())
                    .and_then(|v| checked_ms(v, gap.clearance_ms()))
                    .ok_or(overflow)?;
                // clear 时间按 post-step 末端记录；lag 从该保守基准计已逝时间，
                // 不再加 dt。只有 lead 需要覆盖 subject 在本 interval 末端 crossing。
                let lag = checked_ms(gap.minimum_lag_ms(), gap.clearance_ms()).ok_or(overflow)?;
                let proof = checked_ms(lead, 1).ok_or(overflow)?;
                horizon = Some(horizon.map_or(proof, |old: u64| old.max(proof)));
                gaps.push(DerivedPolicyGap {
                    required_lead_ms: lead,
                    required_lag_ms: lag,
                });
            }
        }
        Ok(Self {
            selection,
            ordinal,
            gaps: gaps.into_boxed_slice(),
            frontier_proof_horizon_ms: horizon,
        })
    }

    pub(crate) const fn selection(&self) -> WorldPolicySelection {
        self.selection
    }
    pub(crate) fn policy<'a>(&self, revision: &'a SharedNetworkRevision) -> Option<PolicyView<'a>> {
        revision.policy().policy(self.ordinal?)
    }
    pub(crate) fn gaps(&self) -> &[DerivedPolicyGap] {
        &self.gaps
    }
    pub(crate) const fn horizon(&self) -> Option<u64> {
        self.frontier_proof_horizon_ms
    }
}

impl crate::TrafficWorld {
    /// 冷边界的策略身份与法规版本连续性；不接受描述符隐式换选。
    pub(crate) fn validate_cutover_policy(
        &self,
        target: &SharedNetworkRevision,
    ) -> Result<(), crate::CutoverError> {
        if let WorldPolicySelection::Pinned(pin) = self.policy_selection() {
            let before = self.policy().expect("installed policy exists");
            let after = target
                .identity()
                .ordinal(pin.policy)
                .and_then(|ordinal| target.policy().policy(ordinal))
                .ok_or(crate::CutoverError::PolicyInstall(
                    InstallError::UnknownPolicy { policy: pin.policy },
                ))?;
            if before.jurisdiction() != after.jurisdiction()
                || before.regulation_version() != after.regulation_version()
            {
                return Err(crate::CutoverError::PolicyRegulationMismatch);
            }
        }
        Ok(())
    }
}

fn checked_ms(a: u64, b: u64) -> Option<u64> {
    a.checked_add(b)
        .filter(|value| *value <= 9_007_199_254_740_991)
}

#[cfg(test)]
mod tests {
    #[test]
    fn derived_milliseconds_never_exceed_portable_integer_domain() {
        assert_eq!(
            super::checked_ms(9_007_199_254_740_990, 1),
            Some(9_007_199_254_740_991)
        );
        assert_eq!(super::checked_ms(9_007_199_254_740_991, 1), None);
        assert_eq!(super::checked_ms(u64::MAX, 1), None);
    }
}
