//! Maidenhead locators, for the path length a field log wants.
//!
//! A locator names a rectangle: two letters for an 18 × 18 field of 20° × 10°, two digits
//! for a 10 × 10 square of 2° × 1°, and, optionally, two letters for a 24 × 24 subsquare of
//! 5' × 2.5'. The point a locator stands for here is the rectangle's centre, so two
//! stations that gave four characters are placed within half a square of where they are —
//! about 55 km at most, which is what the field log's kilometres are good to.

/// The latitude and longitude, degrees, of the centre of a locator's rectangle; `None`
/// for anything that is not a four- or six-character locator.
#[must_use]
pub fn locator(grid: &str) -> Option<(f64, f64)> {
    let s: Vec<char> = grid.trim().to_ascii_uppercase().chars().collect();
    if s.len() != 4 && s.len() != 6 {
        return None;
    }
    let field = |c: char| {
        ('A'..='R')
            .contains(&c)
            .then(|| f64::from(c as u32 - 'A' as u32))
    };
    let digit = |c: char| c.is_ascii_digit().then(|| f64::from(c as u32 - '0' as u32));
    let sub = |c: char| {
        ('A'..='X')
            .contains(&c)
            .then(|| f64::from(c as u32 - 'A' as u32))
    };
    let mut lon = field(s[0])? * 20.0 - 180.0;
    let mut lat = field(s[1])? * 10.0 - 90.0;
    lon += digit(s[2])? * 2.0;
    lat += digit(s[3])?;
    if s.len() == 6 {
        lon += sub(s[4])? * (2.0 / 24.0) + 1.0 / 24.0;
        lat += sub(s[5])? * (1.0 / 24.0) + 0.5 / 24.0;
    } else {
        lon += 1.0;
        lat += 0.5;
    }
    Some((lat, lon))
}

/// The centre of a four- or six-character square, as `(lat, lon)`, worked in whole
/// twenty-fourths and forty-eighths of a degree and divided once, so that every program reading
/// the square gets the same two numbers to the last bit (`RadioPost`'s RP-4 §4.4 compares them as
/// written): what a beacon's square is heard as (ADR-0142). `None` for anything else.
#[must_use]
pub fn square_centre(grid: &str) -> Option<(f64, f64)> {
    let b = grid.trim().as_bytes();
    if b.len() != 4 && b.len() != 6 {
        return None;
    }
    let letter = |c: u8, most: u8| {
        c.to_ascii_uppercase()
            .checked_sub(b'A')
            .filter(|&v| v <= most)
            .map(i32::from)
    };
    let digit = |c: u8| c.checked_sub(b'0').filter(|&v| v <= 9).map(i32::from);
    let (fx, fy) = (letter(b[0], 17)?, letter(b[1], 17)?);
    let (sx, sy) = (digit(b[2])?, digit(b[3])?);
    if b.len() == 4 {
        return Some((
            f64::from(-90 + fy * 10 + sy) + 0.5,
            f64::from(-180 + fx * 20 + sx * 2 + 1),
        ));
    }
    let (ux, uy) = (letter(b[4], 23)?, letter(b[5], 23)?);
    let lon_24ths = (fx * 20 + sx * 2) * 24 + ux * 2 + 1;
    let lat_48ths = (fy * 10 + sy) * 48 + uy * 2 + 1;
    Some((
        -90.0 + f64::from(lat_48ths) / 48.0,
        -180.0 + f64::from(lon_24ths) / 24.0,
    ))
}

/// The four- or six-character square that holds a position, `None` for a position off the
/// globe or another length. The north pole and the antimeridian stay in the last squares.
#[must_use]
pub fn square_of(lat: f64, lon: f64, chars: usize) -> Option<String> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    if chars != 4 && chars != 6 {
        return None;
    }
    // in whole subsquares from the south-west corner: 24 a square across and up
    let cells = |v: f64, per_degree: f64, most: u32| {
        let whole = (v * per_degree).floor();
        if whole <= 0.0 {
            0
        } else if whole >= f64::from(most) {
            most - 1
        } else {
            // whole is in 0..most, so the cast keeps it
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                whole as u32
            }
        }
    };
    let x = cells(lon + 180.0, 12.0, 4320);
    let y = cells(lat + 90.0, 24.0, 4320);
    let letter = |v: u32, base: u8| char::from(base + u8::try_from(v).unwrap_or(0));
    let digit = |v: u32| char::from(b'0' + u8::try_from(v).unwrap_or(0));
    let mut out = String::with_capacity(chars);
    out.push(letter(x / 240, b'A'));
    out.push(letter(y / 240, b'A'));
    out.push(digit(x % 240 / 24));
    out.push(digit(y % 240 / 24));
    if chars == 6 {
        out.push(letter(x % 24, b'a'));
        out.push(letter(y % 24, b'a'));
    }
    Some(out)
}

/// The great-circle distance between two locators, kilometres, or `None` when either is
/// not one.
#[must_use]
pub fn path_km(a: &str, b: &str) -> Option<f64> {
    let (lat1, lon1) = locator(a)?;
    let (lat2, lon2) = locator(b)?;
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let d_phi = (lat2 - lat1).to_radians();
    let d_lambda = (lon2 - lon1).to_radians();
    let h = (d_phi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (d_lambda / 2.0).sin().powi(2);
    Some(2.0 * 6371.0 * h.sqrt().asin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locator_is_the_centre_of_its_rectangle() {
        let (lat, lon) = locator("EM73").expect("four characters");
        assert!(
            (lat - 33.5).abs() < 1e-9 && (lon - (-85.0)).abs() < 1e-9,
            "{lat} {lon}"
        );
        let (lat, lon) = locator("em73tv").expect("six, any case");
        assert!(
            (lat - 33.895_833).abs() < 1e-5 && (lon - (-84.375)).abs() < 1e-5,
            "{lat} {lon}"
        );
        for bad in ["", "EM7", "EM73t", "ZZ00", "EM7A", "EM73zz", "1234"] {
            assert!(locator(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_path_is_a_great_circle() {
        // London to Boston, the textbook transatlantic path
        let km = path_km("IO91wm", "FN42").expect("both locators");
        assert!((5100.0..5400.0).contains(&km), "{km}");
        assert_eq!(path_km("EM73", "EM73"), Some(0.0));
        assert!(path_km("EM73", "nowhere").is_none());
        let there = path_km("EM73", "FN31").expect("path");
        let back = path_km("FN31", "EM73").expect("path");
        assert!((there - back).abs() < 1e-9);
    }

    #[test]
    fn a_square_is_found_and_centred_as_radiopost_finds_it() {
        // RadioPost's rpmi::position tests, the same cases
        assert_eq!(square_of(34.4621, -83.7514, 6).as_deref(), Some("EM84cl"));
        assert_eq!(square_of(33.78123, -84.38765, 4).as_deref(), Some("EM73"));
        let (lat, lon) = square_centre("EM84cl").expect("a square");
        assert!((lat - (34.0 + 23.0 / 48.0)).abs() < 1e-12, "{lat}");
        assert!((lon + (83.0 + 19.0 / 24.0)).abs() < 1e-12, "{lon}");
        assert_eq!(square_centre("EM73"), Some((33.5, -85.0)));
        assert_eq!(square_centre("em84CL"), square_centre("EM84cl"));
        assert_eq!(square_centre("EM8"), None);
        assert_eq!(square_centre("ZZ99"), None);
        assert_eq!(square_of(89.999, 179.999, 6).as_deref(), Some("RR99xx"));
        assert_eq!(square_of(90.0, 180.0, 6).as_deref(), Some("RR99xx"));
        assert_eq!(square_of(-90.0, -180.0, 6).as_deref(), Some("AA00aa"));
        assert_eq!(square_of(91.0, 0.0, 6), None);
        assert_eq!(square_of(0.0, 0.0, 8), None);
        // and the centre is the field log's, to its own rounding
        let (a, b) = locator("EM84cl").expect("a square");
        assert!((a - lat).abs() < 1e-9 && (b - lon).abs() < 1e-9);
    }
}
