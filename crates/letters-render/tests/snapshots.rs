//! Whole scenes compared with stored pictures, to catch changes no single probe would.

mod common;

use common::*;

#[test]
fn scenes_match_their_snapshots() {
    let images = scene_images();
    for (name, scene, scale) in SCENES {
        let (width, height) = ((240.0 * scale) as u32, (160.0 * scale) as u32);
        let frame = render_with(&list(scene()), &images, width, height, scale);

        assert_snapshot(name, &frame);
    }
}
