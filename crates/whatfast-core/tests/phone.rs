//! The phone numbers as the interface shows them.

use whatfast_core::phone::format;

#[test]
fn an_argentine_landline_has_the_area_and_the_number() {
    assert_eq!(format("543811234567"), "+54 381 123-4567");
}

#[test]
fn the_formatter_is_idempotent() {
    for raw in ["5493816041868", "447723442693", "15551234567"] {
        let once = format(raw);
        assert_eq!(format(&once), once, "{raw} changed the second time");
    }
}
