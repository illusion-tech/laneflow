use super::*;
use laneflow_spatial::HandoffPoseInput;

impl LaneFlowSession {
    pub(super) fn collect_handoff_poses(
        &mut self,
        selected: bool,
    ) -> Result<(), LaneFlowAdapterError> {
        self.pose_handoff_scratch.clear();
        let active = self.world.committed_handoff_vehicles();
        if active.len() == 0 || self.pose_vehicle_scratch.is_empty() {
            return Ok(());
        }
        if !selected {
            self.pose_handoff_members.clear();
            self.pose_handoff_members.extend(active);
        }
        for (index, &vehicle) in self.pose_vehicle_scratch.iter().enumerate() {
            if !selected && !self.pose_handoff_members.contains(&vehicle) {
                continue;
            }
            if let Some(geometry) = self
                .world
                .committed_handoff_geometry(vehicle)
                .map_err(|source| LaneFlowAdapterError::HandoffPose { vehicle, source })?
            {
                self.pose_handoff_scratch
                    .push((self.pose_scratch[index].record(), geometry));
            }
        }
        Ok(())
    }
}

pub(super) fn spatial_input(
    (record, geometry): &(PoseRecordId, CommittedHandoffGeometry),
) -> HandoffPoseInput<'_> {
    HandoffPoseInput {
        input_index: record.raw() as usize,
        revision: geometry.revision,
        frame: geometry.frame,
        pose: &geometry.pose,
        front: geometry.footprint.front.addresses().next(),
        front_ground_mm: geometry.footprint.front_ground_mm,
    }
}
