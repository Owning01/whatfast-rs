//! Port of `tauri/tests/media-crop.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use whatfast_core::media_crop::{
    CropRect, PixelSize, Rotation, crop_in_rotated_space, normalize_crop_rect, parse_rotation,
    plan_edit, rotated_size,
};

const SOURCE: PixelSize = PixelSize {
    width: 200.0,
    height: 100.0,
};

const RECT: CropRect = CropRect {
    x: 20.0,
    y: 10.0,
    width: 40.0,
    height: 30.0,
};

/// Inverse of the clockwise rotation, used to check the round trip.
fn unrotate_point(x: f64, y: f64, source: PixelSize, rotation: Rotation) -> (f64, f64) {
    match rotation {
        Rotation::Deg90 => (y, source.height - x),
        Rotation::Deg180 => (source.width - x, source.height - y),
        Rotation::Deg270 => (source.width - y, x),
        Rotation::Deg0 => (x, y),
    }
}

fn corners(rect: CropRect) -> [(f64, f64); 4] {
    [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x, rect.y + rect.height),
        (rect.x + rect.width, rect.y + rect.height),
    ]
}

#[test]
fn a_reversed_drag_becomes_a_positive_rectangle_anchored_at_the_dragged_corner() {
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: 80.0,
                y: 60.0,
                width: -40.0,
                height: -30.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 40.0,
            y: 30.0,
            width: 40.0,
            height: 30.0,
        },
    );
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: 60.0,
                y: 40.0,
                width: -40.0,
                height: 30.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 20.0,
            y: 40.0,
            width: 40.0,
            height: 30.0,
        },
    );
}

#[test]
fn a_rectangle_that_leaves_the_source_is_clamped_to_its_bounds() {
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: 180.0,
                y: 90.0,
                width: 100.0,
                height: 100.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 180.0,
            y: 90.0,
            width: 20.0,
            height: 10.0,
        },
    );
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: -10.0,
                y: -10.0,
                width: 30.0,
                height: 30.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 0.0,
            y: 0.0,
            width: 30.0,
            height: 30.0,
        },
    );
}

#[test]
fn a_rectangle_entirely_outside_the_source_collapses_to_one_pixel_at_the_origin() {
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: 500.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
    );
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: -50.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
    );
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: 0.0,
                y: 500.0,
                width: 10.0,
                height: 10.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
    );
}

#[test]
fn a_crop_never_covers_less_than_one_pixel() {
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: 10.0,
                y: 10.0,
                width: 0.0,
                height: 0.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 10.0,
            y: 10.0,
            width: 1.0,
            height: 1.0,
        },
    );
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: 199.0,
                y: 99.0,
                width: 0.0,
                height: 0.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 199.0,
            y: 99.0,
            width: 1.0,
            height: 1.0,
        },
    );
}

#[test]
fn non_finite_rectangle_components_never_leak_into_the_result() {
    let broken = normalize_crop_rect(
        CropRect {
            x: f64::NAN,
            y: f64::INFINITY,
            width: f64::NAN,
            height: f64::NEG_INFINITY,
        },
        SOURCE,
    )
    .unwrap();
    assert_eq!(
        broken,
        CropRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
    );
    for value in [broken.x, broken.y, broken.width, broken.height] {
        assert!(value.is_finite());
    }
    assert_eq!(
        normalize_crop_rect(
            CropRect {
                x: f64::NAN,
                y: 10.0,
                width: 20.0,
                height: 20.0,
            },
            SOURCE
        )
        .unwrap(),
        CropRect {
            x: 0.0,
            y: 10.0,
            width: 20.0,
            height: 20.0,
        },
    );
}

#[test]
fn a_broken_source_size_is_a_programming_error() {
    for size in [
        PixelSize {
            width: -1.0,
            height: 10.0,
        },
        PixelSize {
            width: 10.0,
            height: -1.0,
        },
        PixelSize {
            width: f64::NAN,
            height: 10.0,
        },
        PixelSize {
            width: 10.0,
            height: f64::INFINITY,
        },
    ] {
        assert!(normalize_crop_rect(RECT, size).is_err());
        assert!(rotated_size(size, Rotation::Deg90).is_err());
        assert!(crop_in_rotated_space(RECT, size, Rotation::Deg90).is_err());
        assert!(plan_edit(size, RECT, Rotation::Deg90).is_err());
    }
}

#[test]
fn rotating_by_a_quarter_turn_swaps_the_image_dimensions() {
    assert_eq!(
        rotated_size(SOURCE, Rotation::Deg0).unwrap(),
        PixelSize {
            width: 200.0,
            height: 100.0
        },
    );
    assert_eq!(
        rotated_size(SOURCE, Rotation::Deg180).unwrap(),
        PixelSize {
            width: 200.0,
            height: 100.0
        },
    );
    assert_eq!(
        rotated_size(SOURCE, Rotation::Deg90).unwrap(),
        PixelSize {
            width: 100.0,
            height: 200.0
        },
    );
    assert_eq!(
        rotated_size(SOURCE, Rotation::Deg270).unwrap(),
        PixelSize {
            width: 100.0,
            height: 200.0
        },
    );
}

#[test]
fn the_same_crop_maps_to_the_four_expected_rectangles_in_the_rotated_space() {
    assert_eq!(
        crop_in_rotated_space(RECT, SOURCE, Rotation::Deg0).unwrap(),
        CropRect {
            x: 20.0,
            y: 10.0,
            width: 40.0,
            height: 30.0,
        },
    );
    assert_eq!(
        crop_in_rotated_space(RECT, SOURCE, Rotation::Deg90).unwrap(),
        CropRect {
            x: 60.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        },
    );
    assert_eq!(
        crop_in_rotated_space(RECT, SOURCE, Rotation::Deg180).unwrap(),
        CropRect {
            x: 140.0,
            y: 60.0,
            width: 40.0,
            height: 30.0,
        },
    );
    assert_eq!(
        crop_in_rotated_space(RECT, SOURCE, Rotation::Deg270).unwrap(),
        CropRect {
            x: 10.0,
            y: 140.0,
            width: 30.0,
            height: 40.0,
        },
    );
}

#[test]
fn the_rotated_crop_stays_inside_the_rotated_canvas_for_every_angle() {
    for rotation in Rotation::ANGLES {
        let canvas = rotated_size(SOURCE, rotation).unwrap();
        let crop = crop_in_rotated_space(RECT, SOURCE, rotation).unwrap();
        assert!(crop.x >= 0.0, "x of {rotation:?}");
        assert!(crop.y >= 0.0, "y of {rotation:?}");
        assert!(crop.x + crop.width <= canvas.width, "width of {rotation:?}");
        assert!(
            crop.y + crop.height <= canvas.height,
            "height of {rotation:?}"
        );
        assert!(crop.width >= 1.0 && crop.height >= 1.0);
    }
}

#[test]
fn rotating_a_crop_and_rotating_it_back_returns_the_original_rectangle() {
    for rotation in Rotation::ANGLES {
        let crop = crop_in_rotated_space(RECT, SOURCE, rotation).unwrap();
        let back: Vec<(f64, f64)> = corners(crop)
            .iter()
            .map(|(x, y)| unrotate_point(*x, *y, SOURCE, rotation))
            .collect();
        let min_x = back
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min);
        let max_x = back
            .iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = back
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        let max_y = back
            .iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(
            CropRect {
                x: min_x,
                y: min_y,
                width: max_x - min_x,
                height: max_y - min_y,
            },
            RECT,
            "rotation {rotation:?}"
        );
    }
}

#[test]
fn the_plan_outputs_the_size_of_the_rotated_crop() {
    let plan0 = plan_edit(SOURCE, RECT, Rotation::Deg0).unwrap();
    assert_eq!(plan0.rotation, Rotation::Deg0);
    assert_eq!(
        plan0.output,
        PixelSize {
            width: 40.0,
            height: 30.0
        },
    );
    assert_eq!(
        plan0.crop,
        CropRect {
            x: 20.0,
            y: 10.0,
            width: 40.0,
            height: 30.0,
        },
    );

    let plan90 = plan_edit(SOURCE, RECT, Rotation::Deg90).unwrap();
    assert_eq!(plan90.rotation, Rotation::Deg90);
    assert_eq!(
        plan90.output,
        PixelSize {
            width: 30.0,
            height: 40.0
        },
    );
    assert_eq!(
        plan90.crop,
        CropRect {
            x: 60.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        },
    );

    let plan180 = plan_edit(SOURCE, RECT, Rotation::Deg180).unwrap();
    assert_eq!(
        plan180.output,
        PixelSize {
            width: 40.0,
            height: 30.0
        },
    );

    let plan270 = plan_edit(SOURCE, RECT, Rotation::Deg270).unwrap();
    assert_eq!(
        plan270.output,
        PixelSize {
            width: 30.0,
            height: 40.0
        },
    );
}

#[test]
fn a_plan_normalises_the_requested_crop_before_rotating_it() {
    let plan = plan_edit(
        SOURCE,
        CropRect {
            x: 180.0,
            y: 90.0,
            width: 100.0,
            height: 100.0,
        },
        Rotation::Deg90,
    )
    .unwrap();
    assert_eq!(
        plan.crop,
        CropRect {
            x: 0.0,
            y: 180.0,
            width: 10.0,
            height: 20.0,
        },
    );
    assert_eq!(
        plan.output,
        PixelSize {
            width: 10.0,
            height: 20.0
        },
    );
}

#[test]
fn any_number_of_degrees_parses_into_the_nearest_quarter_turn() {
    assert_eq!(parse_rotation(0.0), Rotation::Deg0);
    assert_eq!(parse_rotation(45.0), Rotation::Deg90);
    assert_eq!(parse_rotation(90.0), Rotation::Deg90);
    assert_eq!(parse_rotation(-45.0), Rotation::Deg0);
    assert_eq!(parse_rotation(-90.0), Rotation::Deg270);
    assert_eq!(parse_rotation(180.0), Rotation::Deg180);
    assert_eq!(parse_rotation(270.0), Rotation::Deg270);
    assert_eq!(parse_rotation(360.0), Rotation::Deg0);
    assert_eq!(parse_rotation(-360.0), Rotation::Deg0);
    assert_eq!(parse_rotation(720.0), Rotation::Deg0);
    assert_eq!(parse_rotation(400.0), Rotation::Deg0);
    assert_eq!(parse_rotation(f64::NAN), Rotation::Deg0);
    assert_eq!(parse_rotation(f64::INFINITY), Rotation::Deg0);
}
