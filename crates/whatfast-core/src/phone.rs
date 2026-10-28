//! The phone numbers as the interface shows them.
//!
//! WhatsApp paints a number with the plus, the calling code and the groups of
//! its country: `+54 9 381 604-1868`. The client received the bare digits of
//! the jid (`5493816041868`) and painted them as one long run of digits, so a
//! contact that was not in the address book had no readable number.

/// The digits of a number, without the plus, the spaces or the dashes.
pub fn digits(raw: &str) -> String {
    raw.chars()
        .filter(|character| character.is_ascii_digit())
        .collect()
}

/// The number with the plus, the calling code and the groups of its country.
///
/// A string that already has the plus and a space comes back as it is, so the
/// formatter can run on a value whose origin is not known. A string without
/// digits, and a number of a country that is not in the table, also come back
/// as they came: a wrong grouping is worse than none.
pub fn format(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || is_grouped(trimmed) {
        return trimmed.to_owned();
    }
    let digits = digits(trimmed);
    // The shortest international number is 8 digits (Uruguay) and the longest
    // is 15 (the limit of E.164).
    if digits.len() < 8 || digits.len() > 15 {
        return trimmed.to_owned();
    }
    let Some((code, national)) = split_country(&digits) else {
        return trimmed.to_owned();
    };
    format!("+{code} {}", group(code, national))
}

/// Whether the number is already written the way the interface wants it.
fn is_grouped(number: &str) -> bool {
    number.starts_with('+') && number.contains(' ')
}

/// The calling code of the number and its national digits.
///
/// A code that matches but whose national length is not one of the lengths of
/// the country is not a match: that is how the numbers of the modern protocol
/// (`265867099152617@lid`) stay as they are instead of passing for a phone.
fn split_country(digits: &str) -> Option<(&'static str, &str)> {
    COUNTRIES.iter().find_map(|(code, lengths)| {
        let national = digits.strip_prefix(code)?;
        lengths
            .contains(&national.len())
            .then_some((*code, national))
    })
}

/// How each country of the table groups its national digits.
///
/// The sizes are counted from the left and must add up to the national length;
/// any other number is left as it is.
fn group(code: &str, national: &str) -> String {
    match code {
        "54" => argentina(national),
        "55" => brazil(national),
        "1" => split(national, &[3, 3, 4], "-"),
        "34" => split(national, &[3, 2, 2, 2], " "),
        "33" => split(national, &[1, 2, 2, 2, 2], " "),
        "39" => split(national, &[3, 3, 4], " "),
        "44" => split(national, &[4, 6], " "),
        "49" => split(national, &[3, 3, 4], " "),
        "51" => split(national, &[3, 3, 3], " "),
        "52" => split(national, &[2, 4, 4], " "),
        "53" => split(national, &[1, 3, 4], " "),
        "56" => split(national, &[1, 4, 4], " "),
        "57" => split(national, &[3, 3, 4], " "),
        "58" => split(national, &[3, 3, 4], " "),
        "351" => split(national, &[3, 3, 3], " "),
        "353" => split(national, &[2, 3, 4], " "),
        "591" => split(national, &[4, 4], " "),
        "593" => split(national, &[2, 3, 4], " "),
        "595" => split(national, &[3, 3, 3], " "),
        "598" => split(national, &[2, 3, 3], " "),
        _ => national.to_owned(),
    }
}

/// The Argentine number: the marker of the mobile, the area code and the
/// number. The capital has two digits of area code and the rest of the country
/// has three.
fn argentina(national: &str) -> String {
    let (marker, rest) = match national.strip_prefix('9') {
        Some(rest) if rest.len() == 10 => ("9 ", rest),
        _ => ("", national),
    };
    if rest.len() != 10 {
        return national.to_owned();
    }
    let area_size = if rest.starts_with("11") { 2 } else { 3 };
    let (area, number) = rest.split_at(area_size);
    let groups = match number.len() {
        8 => format!("{}-{}", &number[..4], &number[4..]),
        7 => format!("{}-{}", &number[..3], &number[3..]),
        _ => return national.to_owned(),
    };
    format!("{marker}{area} {groups}")
}

/// The Brazilian number: two digits of area code and a subscriber of nine
/// digits (mobile) or eight (landline).
fn brazil(national: &str) -> String {
    if national.len() < 10 {
        return national.to_owned();
    }
    let (area, number) = national.split_at(2);
    let groups = match number.len() {
        9 => format!("{}-{}", &number[..5], &number[5..]),
        8 => format!("{}-{}", &number[..4], &number[4..]),
        _ => return national.to_owned(),
    };
    format!("{area} {groups}")
}

/// The national digits in the given sizes, joined with the separator.
fn split(national: &str, sizes: &[usize], separator: &str) -> String {
    if sizes.iter().sum::<usize>() != national.len() {
        return national.to_owned();
    }
    let mut parts = Vec::with_capacity(sizes.len());
    let mut rest = national;
    for size in sizes {
        let (head, tail) = rest.split_at(*size);
        parts.push(head);
        rest = tail;
    }
    parts.join(separator)
}

/// The countries of the table: the calling code and the lengths of the
/// national number it uses. The longer codes come first, so that 598 wins over
/// a shorter code that starts with the same digits.
const COUNTRIES: &[(&str, &[usize])] = &[
    ("351", &[9]),
    ("353", &[9]),
    ("591", &[8]),
    ("593", &[9]),
    ("595", &[9]),
    ("598", &[8]),
    ("33", &[9]),
    ("34", &[9]),
    ("39", &[9, 10]),
    ("44", &[10]),
    ("49", &[10, 11]),
    ("51", &[9]),
    ("52", &[10]),
    ("53", &[8]),
    ("54", &[10, 11]),
    ("55", &[10, 11]),
    ("56", &[9]),
    ("57", &[10]),
    ("58", &[10]),
    ("1", &[10]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digits_of_a_number_lose_the_plus_and_the_spaces() {
        assert_eq!(digits("+54 9 381 604-1868"), "5493816041868");
        assert_eq!(digits(""), "");
    }

    #[test]
    fn an_argentine_mobile_has_the_marker_and_the_area() {
        assert_eq!(format("5493816041868"), "+54 9 381 604-1868");
        assert_eq!(format("5491154726343"), "+54 9 11 5472-6343");
        assert_eq!(format("+5493816041868"), "+54 9 381 604-1868");
    }

    #[test]
    fn an_argentine_landline_has_the_area_and_the_number() {
        assert_eq!(format("543811234567"), "+54 381 123-4567");
    }

    #[test]
    fn the_number_of_another_country_gets_the_groups_of_its_country() {
        assert_eq!(format("447723442693"), "+44 7723 442693");
        assert_eq!(format("15551234567"), "+1 555-123-4567");
        assert_eq!(format("34612345678"), "+34 612 34 56 78");
        assert_eq!(format("5511912345678"), "+55 11 91234-5678");
        assert_eq!(format("56912345678"), "+56 9 1234 5678");
        assert_eq!(format("59899123456"), "+598 99 123 456");
    }

    #[test]
    fn a_number_already_grouped_stays_as_it_is() {
        assert_eq!(format("+54 9 381 604-1868"), "+54 9 381 604-1868");
        assert_eq!(format("  +1 555-123-4567  "), "+1 555-123-4567");
    }

    #[test]
    fn a_string_without_digits_or_with_a_number_of_no_country_stays_as_it_came() {
        assert_eq!(format(""), "");
        assert_eq!(format("Ana"), "Ana");
        assert_eq!(format("265867099152617"), "265867099152617");
        assert_eq!(format("162332734951579"), "162332734951579");
        assert_eq!(format("1234"), "1234");
    }

    #[test]
    fn the_formatter_is_idempotent() {
        for raw in ["5493816041868", "447723442693", "15551234567"] {
            let once = format(raw);
            assert_eq!(format(&once), once, "{raw} changed the second time");
        }
    }
}
