//! The regulatory policy against 47 CFR Part 97, edge by edge (ADR-0018).

use super::*;

fn us() -> Policy {
    Policy::from_setting("us-fcc-part97")
}

/// The United States profile read the literal way (a ratio of powers), for the paths the
/// conservative reading closes — a profile that reads bandwidth so is one data change away.
fn us_power_reading() -> Policy {
    let mut profile = profile::load("us-fcc-part97").expect("loads");
    profile.bandwidth.reading = Reading::Power;
    Policy::Rules(Arc::new(profile))
}

fn station(
    dial: f64,
    control: ControlMode,
    license: LicenseClass,
    sideband: Sideband,
) -> Situation {
    Situation {
        dial_hz: Some(dial),
        dial_source: Some(DialSource::Radio),
        sideband: Some(sideband),
        control: Some(control),
        license: Some(license),
        itu_region: 2,
        margin_hz: 0.0,
        band_plan: false,
        power_w: None,
    }
}

fn local(dial: f64) -> Situation {
    station(dial, ControlMode::Local, LicenseClass::Extra, Sideband::Usb)
}

/// A data emission of known audio edges, the same under both readings: exact arithmetic.
fn data(low: f64, high: f64, direction: Direction) -> Transmission {
    Transmission {
        what: "a test emission".into(),
        kind: EmissionKind::Data,
        audio: Edges {
            power: [low, high],
            spectral: [low, high],
        },
        direction,
    }
}

/// Aether's own data frames at a rung of an air, as measured.
fn rung(bandwidth: usize, rung: usize, direction: Direction) -> Transmission {
    let air = occupancy::air(bandwidth).expect("measured");
    Transmission {
        what: format!("rung {rung} of the {bandwidth} Hz air"),
        kind: EmissionKind::Data,
        audio: air.rung(rung).expect("a rung"),
        direction,
    }
}

fn verdict(policy: &Policy, s: &Situation, tx: &Transmission) -> (Verdict, &'static str) {
    let d = policy.evaluate(s, tx);
    (d.verdict, d.code)
}

const LEGAL: (Verdict, &str) = (Verdict::Legal, "permitted");

// ── the RF range ─────────────────────────────────────────────────────

#[test]
fn the_rf_range_is_the_audio_on_the_dial_upper_or_mirrored_lower() {
    assert_eq!(
        rf_range(7_080_000.0, Sideband::Usb, 1000.0, 2000.0),
        (7_081_000.0, 7_082_000.0)
    );
    assert_eq!(
        rf_range(7_080_000.0, Sideband::Lsb, 1000.0, 2000.0),
        (7_078_000.0, 7_079_000.0)
    );
    // the dial is not the centre: a 2 300 Hz waveform centred at 1 500 Hz of audio
    let wide = occupancy::air(2300)
        .expect("measured")
        .rung(10)
        .expect("rung");
    let (lo, hi) = wide.range(Reading::Wider);
    let (rf_lo, rf_hi) = rf_range(7_080_000.0, Sideband::Usb, lo, hi);
    assert!((f64::midpoint(rf_lo, rf_hi) - 7_081_500.0).abs() < 10.0);
    assert!(rf_lo > 7_080_000.0, "all of it above the dial on USB");
}

// ── the edges of a data segment, to the hertz ────────────────────────

#[test]
fn a_data_segment_holds_a_signal_to_its_last_hertz_and_no_further() {
    let policy = us();
    let tx = data(1000.0, 2000.0, Direction::Originate);
    // 40 m data: 7.000–7.125 MHz; on USB the signal is dial + 1000 … dial + 2000
    let at = |dial: f64| verdict(&policy, &local(dial), &tx);
    assert_eq!(
        at(7_123_000.0),
        LEGAL,
        "the upper occupied edge exactly on the segment's"
    );
    assert_eq!(at(7_122_999.0), LEGAL, "one hertz inside");
    assert_eq!(
        at(7_123_001.0),
        (Verdict::Blocked, "outside_data_segment"),
        "one hertz beyond"
    );
    assert_eq!(
        at(6_999_000.0),
        LEGAL,
        "the lower occupied edge exactly on the segment's"
    );
    assert_eq!(at(6_999_001.0), LEGAL, "one hertz inside");
    assert_eq!(
        at(6_998_999.0).0,
        Verdict::Blocked,
        "one hertz below the band"
    );
    assert_eq!(
        at(6_900_000.0),
        (Verdict::Blocked, "out_of_band"),
        "well below"
    );
    // the 75 m phone band has no data at all
    assert_eq!(at(3_800_000.0), (Verdict::Blocked, "no_data_here"));
}

#[test]
fn on_the_lower_sideband_the_signal_is_mirrored_below_the_dial() {
    let policy = us();
    let tx = data(1000.0, 2000.0, Direction::Originate);
    let lsb = |dial: f64| {
        verdict(
            &policy,
            &station(dial, ControlMode::Local, LicenseClass::Extra, Sideband::Lsb),
            &tx,
        )
    };
    // dial − 2000 … dial − 1000
    assert_eq!(
        lsb(7_126_000.0),
        LEGAL,
        "the upper occupied edge exactly on the segment's"
    );
    assert_eq!(lsb(7_126_001.0).0, Verdict::Blocked, "one hertz beyond");
    assert_eq!(
        lsb(7_002_000.0),
        LEGAL,
        "the lower edge exactly on the band's"
    );
    assert_eq!(lsb(7_001_999.0).0, Verdict::Blocked);
}

#[test]
fn a_nominally_legal_dial_is_refused_when_the_signal_crosses_the_edge() {
    let policy = us();
    // the dial is inside the 40 m data segment; the 2 300 Hz waveform is not
    let tx = rung(2300, 10, Direction::Originate);
    let (_, audio_hi) = tx.audio.range(Reading::Wider);
    let beyond = 7_123_000.0 + audio_hi - 7_125_000.0;
    let d = policy.evaluate(&local(7_123_000.0), &tx);
    assert_eq!(d.verdict, Verdict::Blocked);
    assert_eq!(d.code, "outside_data_segment");
    assert!(beyond > 700.0, "{beyond}");
    assert!(
        d.summary
            .contains(&format!("extends {beyond:.0} Hz beyond")),
        "{}",
        d.summary
    );
    assert!(
        d.detail.contains("upper edge of the 40 m data segment"),
        "{}",
        d.detail
    );
    assert!(
        d.rf_high_hz
            .is_some_and(|hi| (hi - (7_125_000.0 + beyond)).abs() < 1e-6)
    );
    // the 500 Hz waveform fits there
    assert_eq!(
        verdict(
            &policy,
            &local(7_123_000.0),
            &rung(500, 10, Direction::Originate)
        ),
        LEGAL
    );
}

#[test]
fn the_margin_is_kept_at_the_edges() {
    let policy = us();
    let tx = data(1000.0, 2000.0, Direction::Originate);
    let mut s = local(7_123_000.0);
    s.margin_hz = 50.0;
    let d = policy.evaluate(&s, &tx);
    assert_eq!(d.code, "outside_data_segment");
    assert!(d.summary.contains("too close"), "{}", d.summary);
    s.dial_hz = Some(7_122_950.0);
    assert_eq!(verdict(&policy, &s, &tx), LEGAL);
}

// ── both waveforms, both operator controls ───────────────────────────

#[test]
fn an_operator_controls_the_station_locally_or_remotely_under_the_same_rules() {
    let policy = us();
    for control in [ControlMode::Local, ControlMode::Remote] {
        let s = station(14_080_000.0, control, LicenseClass::General, Sideband::Usb);
        for (bandwidth, top) in [(500, 14), (2300, 19)] {
            for r in [0, top] {
                for direction in [Direction::Originate, Direction::Respond] {
                    assert_eq!(
                        verdict(&policy, &s, &rung(bandwidth, r, direction)),
                        LEGAL,
                        "{bandwidth} {r} {control:?}"
                    );
                }
            }
        }
        // §97.221(c)'s 500 Hz is no condition on an operator's station
        let wide = policy.evaluate(&s, &rung(2300, 19, Direction::Originate));
        assert!(wide.bandwidth_hz > 2400.0 && wide.allowed());
    }
}

// ── automatic control ────────────────────────────────────────────────

fn automatic(dial: f64) -> Situation {
    station(
        dial,
        ControlMode::Automatic,
        LicenseClass::General,
        Sideband::Usb,
    )
}

#[test]
fn an_automatic_station_uses_either_waveform_inside_the_97_221_b_segments() {
    let policy = us();
    // 14.1005–14.112 MHz
    let s = automatic(14_103_000.0);
    for (bandwidth, r) in [(500, 5), (500, 14), (2300, 10), (2300, 19)] {
        for direction in [Direction::Originate, Direction::Respond] {
            let d = policy.evaluate(&s, &rung(bandwidth, r, direction));
            assert_eq!(
                (d.verdict, d.code),
                (Verdict::Legal, "automatic_segment"),
                "{bandwidth} {r}"
            );
            assert!(d.summary.contains("§97.221(b)"));
            assert_eq!(d.automatic_segment.map(|a| a.low_hz), Some(14_100_500.0));
        }
    }
    // a 2 300 Hz signal must fit the segment whole: 14.095–14.0995 is 4.5 kHz, and the
    // beacon gap above it is not in it
    let d = policy.evaluate(
        &automatic(14_097_500.0),
        &rung(2300, 10, Direction::Respond),
    );
    assert_eq!(d.code, "automatic_bandwidth", "{}", d.detail);
}

#[test]
fn outside_the_segments_an_automatic_station_may_only_answer_and_only_narrowly() {
    let policy = us();
    let s = automatic(14_080_000.0); // a data segment, not a §97.221(b) segment
    // a 400 Hz emission answering a station under local or remote control: §97.221(c)
    let narrow_answer = policy.evaluate(&s, &data(1300.0, 1700.0, Direction::Respond));
    assert_eq!(
        (narrow_answer.verdict, narrow_answer.code),
        (Verdict::Legal, "automatic_response")
    );
    // the same emission starting an exchange
    let narrow_call = policy.evaluate(&s, &data(1300.0, 1700.0, Direction::Originate));
    assert_eq!(narrow_call.code, "automatic_originate");
    assert!(
        narrow_call.summary.contains("may only respond"),
        "{}",
        narrow_call.summary
    );
    // the 2 300 Hz waveform either way
    for direction in [Direction::Respond, Direction::Originate] {
        let d = policy.evaluate(&s, &rung(2300, 10, direction));
        assert_eq!(d.code, "automatic_bandwidth");
        assert!(
            d.summary.contains("outside the §97.221(b)"),
            "{}",
            d.summary
        );
    }
    // an operator testing the station at the control point is not automatic control
    assert_eq!(
        verdict(&policy, &s, &rung(2300, 10, Direction::Operator)),
        LEGAL
    );
}

#[test]
fn the_500_hz_air_is_wider_than_500_hz_and_the_tone_floor_depends_on_the_reading() {
    // measured, not named: the narrow air's OFDM rungs exceed 500 Hz however the rule is
    // read, and the tone floor is under it as a ratio of powers but over it as a spectral
    // level — the conservative profile refuses both outside §97.221(b)
    let s = automatic(14_080_000.0);
    let conservative = us();
    let literal = us_power_reading();
    for policy in [&conservative, &literal] {
        assert_eq!(
            policy.evaluate(&s, &rung(500, 5, Direction::Respond)).code,
            "automatic_bandwidth"
        );
    }
    assert_eq!(
        conservative
            .evaluate(&s, &rung(500, 0, Direction::Respond))
            .code,
        "automatic_bandwidth"
    );
    assert_eq!(
        literal.evaluate(&s, &rung(500, 0, Direction::Respond)).code,
        "automatic_response"
    );
    assert_eq!(
        literal
            .evaluate(&s, &rung(500, 0, Direction::Originate))
            .code,
        "automatic_originate"
    );
}

#[test]
fn an_automatic_answer_never_climbs_past_500_hz() {
    // the ceiling the link's rate control is held to: however good the path
    let is_floor = |bandwidth: usize| move |r: usize| r < if bandwidth == 2300 { 6 } else { 4 };
    let s = automatic(14_080_000.0);
    let wide = occupancy::air(2300).expect("measured");
    let literal = us_power_reading();
    let ceiling = literal.ceiling(&s, wide, Direction::Respond, &is_floor(2300));
    assert_eq!(
        ceiling.rung,
        Some(1),
        "the tone floor's two rungs, and no further"
    );
    assert_eq!(
        ceiling.limit.as_ref().map(|d| d.code),
        Some("automatic_bandwidth")
    );
    // the conservative reading allows nothing there
    assert_eq!(
        us().ceiling(&s, wide, Direction::Respond, &is_floor(2300))
            .rung,
        None
    );
    // inside §97.221(b), everything
    let inside = us().ceiling(
        &automatic(14_103_000.0),
        wide,
        Direction::Respond,
        &is_floor(2300),
    );
    assert_eq!(inside.rung, Some(19));
    // an operator's station near a segment edge: the rungs that fit, and no more
    let edge = us().ceiling(
        &local(7_122_500.0),
        wide,
        Direction::Originate,
        &is_floor(2300),
    );
    assert_eq!(
        edge.rung,
        Some(3),
        "the fast tones up to 1.1 kHz fit, the 1.6 kHz ones do not"
    );
    assert_eq!(edge.limit.map(|d| d.code), Some("outside_data_segment"));
}

// ── 60 m ─────────────────────────────────────────────────────────────

#[test]
fn sixty_metres_is_four_channels_and_a_segment() {
    let policy = us();
    let general =
        |dial: f64, sideband| station(dial, ControlMode::Local, LicenseClass::General, sideband);
    let wide = rung(2300, 10, Direction::Originate);
    // on the 5348 kHz channel: the dial on the carrier, 1.5 kHz below the centre
    let on = policy.evaluate(&general(5_346_500.0, Sideband::Usb), &wide);
    assert_eq!(
        (on.verdict, on.code),
        (Verdict::Legal, "sixty_channel"),
        "{}",
        on.detail
    );
    assert_eq!(
        policy
            .evaluate(&general(5_346_550.0, Sideband::Usb), &wide)
            .code,
        "sixty_channel"
    );
    assert_eq!(
        policy
            .evaluate(&general(5_346_550.0, Sideband::Usb), &wide)
            .verdict,
        Verdict::Blocked,
        "50 Hz off the carrier"
    );
    assert_eq!(
        policy
            .evaluate(&general(5_349_500.0, Sideband::Lsb), &wide)
            .verdict,
        Verdict::Blocked,
        "LSB"
    );
    // in the segment, anywhere it fits
    assert_eq!(
        policy
            .evaluate(&general(5_355_000.0, Sideband::Usb), &wide)
            .code,
        "sixty_segment"
    );
    assert_eq!(
        policy
            .evaluate(&general(5_365_000.0, Sideband::Usb), &wide)
            .verdict,
        Verdict::Blocked
    );
    // between channels
    assert_eq!(
        policy
            .evaluate(&general(5_380_000.0, Sideband::Usb), &wide)
            .code,
        "sixty_off_channel"
    );
    // CW on the centre; a tune tone nowhere on 60 m; no automatic control
    let id = Transmission {
        what: "the Morse identifier".into(),
        kind: EmissionKind::Cw,
        audio: Edges::tone(1500.0, occupancy::CW_HALF_WIDTH_HZ),
        direction: Direction::Originate,
    };
    assert!(
        policy
            .evaluate(&general(5_346_500.0, Sideband::Usb), &id)
            .allowed()
    );
    let tune = Transmission {
        kind: EmissionKind::Test,
        what: "the tune tone".into(),
        ..id.clone()
    };
    assert_eq!(
        policy
            .evaluate(&general(5_346_500.0, Sideband::Usb), &tune)
            .code,
        "sixty_emission"
    );
    let auto = station(
        5_346_500.0,
        ControlMode::Automatic,
        LicenseClass::General,
        Sideband::Usb,
    );
    assert_eq!(
        policy
            .evaluate(&auto, &rung(500, 0, Direction::Respond))
            .code,
        "automatic_excluded"
    );
    // a Technician has no 60 m
    let tech = station(
        5_346_500.0,
        ControlMode::Local,
        LicenseClass::Technician,
        Sideband::Usb,
    );
    assert_eq!(policy.evaluate(&tech, &wide).code, "privilege");
}

// ── license classes ──────────────────────────────────────────────────

#[test]
fn a_license_class_reaches_only_its_privileges() {
    let policy = us();
    let class = |dial: f64, license| station(dial, ControlMode::Local, license, Sideband::Usb);
    let wide = rung(2300, 10, Direction::Originate);
    // 7.020 MHz is Extra only
    assert_eq!(
        verdict(&policy, &class(7_018_000.0, LicenseClass::Extra), &wide),
        LEGAL
    );
    assert_eq!(
        verdict(&policy, &class(7_018_000.0, LicenseClass::General), &wide),
        (Verdict::Blocked, "privilege")
    );
    assert_eq!(
        verdict(&policy, &class(7_080_000.0, LicenseClass::General), &wide),
        LEGAL
    );
    // a Technician's 40 m is CW only; their HF data is 28.0–28.3 MHz
    let tech = policy.evaluate(&class(7_080_000.0, LicenseClass::Technician), &wide);
    assert_eq!(tech.code, "privilege");
    assert!(tech.summary.contains("only CW"), "{}", tech.summary);
    assert_eq!(
        verdict(
            &policy,
            &class(28_120_000.0, LicenseClass::Technician),
            &wide
        ),
        LEGAL
    );
    // CW anywhere in the privileges
    let id = Transmission {
        what: "the Morse identifier".into(),
        kind: EmissionKind::Cw,
        audio: Edges::tone(1500.0, occupancy::CW_HALF_WIDTH_HZ),
        direction: Direction::Originate,
    };
    assert!(
        policy
            .evaluate(&class(7_080_000.0, LicenseClass::Technician), &id)
            .allowed()
    );
    assert!(
        policy
            .evaluate(&class(14_300_000.0, LicenseClass::General), &id)
            .allowed(),
        "CW in the phone segment"
    );
    assert!(
        !policy
            .evaluate(&class(14_300_000.0, LicenseClass::General), &wide)
            .allowed(),
        "but no data there"
    );
}

// ── the voluntary band plan ──────────────────────────────────────────

#[test]
fn the_band_plan_advises_and_never_forbids() {
    let policy = us();
    let wide = rung(2300, 10, Direction::Originate);
    let mut s = local(14_078_000.0);
    s.band_plan = true;
    let inside = policy.evaluate(&s, &wide);
    assert_eq!(inside.verdict, Verdict::Legal);
    assert!(
        inside
            .guidance
            .as_deref()
            .is_some_and(|g| g.contains("band plan"))
    );
    s.dial_hz = Some(14_030_000.0); // legal, outside the customary data area
    let outside = policy.evaluate(&s, &wide);
    assert_eq!(outside.verdict, Verdict::Warning);
    assert!(outside.allowed());
    assert!(
        outside.summary.contains("legal under Part 97"),
        "{}",
        outside.summary
    );
    assert!(!outside.summary.contains("BLOCKED"));
    s.band_plan = false;
    assert_eq!(
        policy.evaluate(&s, &wide).verdict,
        Verdict::Legal,
        "guidance off"
    );
}

// ── nothing is guessed ───────────────────────────────────────────────

#[test]
fn what_cannot_be_known_is_a_refusal() {
    let policy = us();
    let wide = rung(2300, 10, Direction::Originate);
    let base = local(14_080_000.0);
    let without = |f: fn(&mut Situation)| {
        let mut s = base.clone();
        f(&mut s);
        policy.evaluate(&s, &wide).code
    };
    assert_eq!(without(|s| s.dial_hz = None), "no_dial");
    assert_eq!(without(|s| s.sideband = None), "no_sideband");
    assert_eq!(without(|s| s.control = None), "no_control");
    assert_eq!(without(|s| s.license = None), "no_license");
    assert_eq!(without(|s| s.itu_region = 3), "region");
    assert_eq!(Policy::Unset.evaluate(&base, &wide).code, "no_profile");
    let broken = Policy::from_setting("xx-nowhere");
    assert!(matches!(broken, Policy::Broken(_)));
    assert_eq!(broken.evaluate(&base, &wide).code, "profile_broken");
    // the operator's own word that no profile applies is not a guess
    assert!(
        Policy::from_setting("none")
            .evaluate(&base, &wide)
            .allowed()
    );
    // and only the policy makes leave to transmit
    assert!(policy.authorize(&base, &wide).is_ok());
    assert!(Policy::Unset.authorize(&base, &wide).is_err());
}

// ── safe dials ───────────────────────────────────────────────────────

#[test]
fn the_safe_dial_range_is_the_segment_less_the_signal() {
    let policy = us();
    let wide = occupancy::air(2300)
        .expect("measured")
        .rung(10)
        .expect("rung");
    let (lo, hi) = wide.range(Reading::Wider);
    let s = station(
        0.0,
        ControlMode::Local,
        LicenseClass::General,
        Sideband::Usb,
    );
    let dials = policy.safe_dials(&s, wide, Direction::Originate);
    let forty = dials.iter().find(|d| d.band == "40 m").expect("40 m");
    // General on 40 m data: 7.025–7.125 MHz
    assert!((forty.dial_low_hz - (7_025_000.0 - lo)).abs() < 1e-6);
    assert!((forty.dial_high_hz - (7_125_000.0 - hi)).abs() < 1e-6);
    // at either end the signal sits exactly on the segment's edges
    for dial in [forty.dial_low_hz, forty.dial_high_hz] {
        let mut here = s.clone();
        here.dial_hz = Some(dial);
        assert!(
            policy
                .evaluate(&here, &rung(2300, 10, Direction::Originate))
                .allowed()
        );
    }
    // the 60 m channels, each a single dial
    assert!(
        dials
            .iter()
            .any(|d| d.band == "60 m" && (d.dial_low_hz - 5_346_500.0).abs() < 1e-6)
    );
    // an automatic station: the §97.221(b) segments only
    let auto = station(
        0.0,
        ControlMode::Automatic,
        LicenseClass::General,
        Sideband::Usb,
    );
    let auto_dials = policy.safe_dials(&auto, wide, Direction::Respond);
    assert!(!auto_dials.is_empty());
    assert!(
        auto_dials.iter().all(|d| d.rule == "§97.221(b)"),
        "{auto_dials:?}"
    );
    assert!(auto_dials.iter().all(|d| d.band != "60 m"));
}
