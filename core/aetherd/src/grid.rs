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
}
