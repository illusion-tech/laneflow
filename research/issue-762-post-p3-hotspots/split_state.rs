// #810：公共标量调用的冷状态与 IIDM 输入；不收集四车批次。
#[derive(Clone, Copy)]
struct PreparedActiveMotion<'a> {
    state: VehicleState,
    compiled: &'a CompiledRoute,
    lengths: &'a [u32],
    speed_limits: &'a [u32],
    cursor: usize,
    edge: LaneEdgeOrdinal,
    profile: VehicleProfileView,
    leader_gap: Option<i64>,
    route_end: BoundedDistance,
    reach: Option<MotionReach>,
    parking: Option<(ParkingReservation, BoundedDistance)>,
    movement_stop: Option<BoundedDistance>,
    waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    conflict_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    delta_s: f32,
    desired_mm_s: u32,
}

#[derive(Clone, Copy)]
struct IidmInput {
    speed: f32,
    desired: f32,
    gap: Option<f32>,
    min_gap: f32,
    headway: f32,
    accel: f32,
    comfort: f32,
    emergency: f32,
    delta: f32,
}

impl IidmInput {
    const EMPTY: Self = Self {
        speed: 0.0,
        desired: 1.0,
        gap: None,
        min_gap: 0.0,
        headway: 1.0,
        accel: 1.0,
        comfort: 1.0,
        emergency: 1.0,
        delta: 0.033,
    };

    fn scalar(self) -> Option<(f32, f32)> {
        iidm_step(
            self.speed,
            self.desired,
            self.gap,
            self.min_gap,
            self.headway,
            self.accel,
            self.comfort,
            self.emergency,
            self.delta,
        )
    }
}
