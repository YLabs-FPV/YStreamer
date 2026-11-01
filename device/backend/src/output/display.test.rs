use super::*;

fn plane(id: u32, drives_crtc: bool, cursor: bool, nv12: bool) -> PlaneFacts {
    PlaneFacts {
        id,
        drives_crtc,
        cursor,
        nv12,
    }
}

/// The Pi 4 (vc4): the primary plane comes first and takes NV12, which is
/// what kmssink picks by itself
#[test]
fn the_pi_keeps_its_first_plane() {
    let planes = [
        plane(30, true, false, true),
        plane(40, true, false, true),
        plane(50, true, true, true),
        plane(60, false, false, true),
    ];
    assert_eq!(choose_video_plane(&planes), Some(30));
}

/// The RK3568 (ROCK 3A), as its 6.1 kernel lists the planes: RGB-only
/// "Smart" windows, "Esmart0" typed as a cursor, "Cluster" windows without
/// NV12, and "Esmart1" at 112
#[test]
fn the_rock_3a_gets_its_nv12_overlay() {
    let planes = [
        plane(54, true, false, false),
        plane(72, true, true, true),
        plane(96, true, false, false),
        plane(104, true, false, false),
        plane(112, true, false, true),
        plane(120, true, false, true),
    ];
    assert_eq!(choose_video_plane(&planes), Some(112));
}

#[test]
fn another_controllers_planes_do_not_count() {
    let planes = [plane(10, false, false, true), plane(20, true, false, true)];
    assert_eq!(choose_video_plane(&planes), Some(20));
    assert_eq!(choose_video_plane(&[plane(10, false, false, true)]), None);
    assert_eq!(choose_video_plane(&[]), None);
}

#[test]
fn logo_zpos_stays_in_the_drivers_range() {
    // vc4 allows it
    assert_eq!(clamp_zpos(10, Some((0, 15))), 10);
    // A driver with fewer levels gets the top one
    assert_eq!(clamp_zpos(10, Some((0, 5))), 5);
    assert_eq!(clamp_zpos(10, Some((11, 15))), 11);
    // No range known: as before
    assert_eq!(clamp_zpos(10, None), 10);
    assert_eq!(clamp_zpos(10, Some((9, 2))), 10);
}
