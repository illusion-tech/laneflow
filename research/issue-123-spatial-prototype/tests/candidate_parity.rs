use std::mem::{align_of, size_of};

use euclid::{Point3D, Vector3D};
use glam::DVec3;
use laneflow_spatial_research::{CanonicalPoint3, CanonicalVector3};

enum CanonicalSpace {}

#[test]
fn owned_euclid_and_glam_match_basic_f64_vector_math() {
    let owned_start = CanonicalPoint3::new(1.0, 2.0, 3.0);
    let owned_end = CanonicalPoint3::new(4.0, 6.0, 3.0);
    let owned_delta = owned_end - owned_start;

    let euclid_start = Point3D::<f64, CanonicalSpace>::new(1.0, 2.0, 3.0);
    let euclid_end = Point3D::<f64, CanonicalSpace>::new(4.0, 6.0, 3.0);
    let euclid_delta: Vector3D<f64, CanonicalSpace> = euclid_end - euclid_start;

    let glam_start = DVec3::new(1.0, 2.0, 3.0);
    let glam_end = DVec3::new(4.0, 6.0, 3.0);
    let glam_delta = glam_end - glam_start;

    assert_eq!(owned_delta, CanonicalVector3::new(3.0, 4.0, 0.0));
    assert_eq!(owned_delta.length(), 5.0);
    assert_eq!(euclid_delta.length(), 5.0);
    assert_eq!(glam_delta.length(), 5.0);
}

#[test]
fn candidate_point_and_vector_layouts_do_not_justify_public_type_leakage() {
    let owned_size = size_of::<CanonicalPoint3>();
    assert_eq!(size_of::<CanonicalVector3>(), owned_size);
    assert_eq!(size_of::<Point3D<f64, CanonicalSpace>>(), owned_size);
    assert_eq!(size_of::<Vector3D<f64, CanonicalSpace>>(), owned_size);
    assert_eq!(size_of::<DVec3>(), owned_size);

    let owned_alignment = align_of::<CanonicalPoint3>();
    assert_eq!(align_of::<CanonicalVector3>(), owned_alignment);
    assert_eq!(align_of::<Point3D<f64, CanonicalSpace>>(), owned_alignment);
    assert_eq!(align_of::<Vector3D<f64, CanonicalSpace>>(), owned_alignment);
    assert_eq!(align_of::<DVec3>(), owned_alignment);
}
