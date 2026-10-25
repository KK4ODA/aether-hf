//! What a host program that starts Aether as its "VARA program" gets to watch.
//!
//! RMS Trimode starts its modem the way it starts VARA, `<path> TCPIP 127.0.0.1 8300`, watches
//! the process, and restarts the modem by killing it ("Trying to stop Vara TNC") and starting it
//! again. Pointed at `aether-hf.exe`, the process it started was the window: each restart closed
//! Aether's window and opened another, and the modem the window had started ran on with no
//! window watching it (KK4ODA-1, 2026-10-25: from 23:57 to 03:45 the same modem through six
//! kills). Started that way, this program is a stand-in instead: no window, it starts Aether as
//! a process of its own only when no modem answers, and it stays while the modem runs. A kill
//! ends the stand-in and nothing else; the modem stopping ends the stand-in a minute later, as
//! VARA's process would end, so the host program starts it again as it would VARA.

use std::time::{Duration, Instant};

/// How long the stand-in waits for a modem that said it is stopping to be started again — a
/// profile switch, a setting that needs a restart — before starting Aether: such a modem is
/// back within a second or a few, and a second one started meanwhile would be nobody's. A
/// modem that said nothing of the kind is not waited for: the host program connects about
/// three seconds after it starts the stand-in, and finding no port it starts it again.
const START_GRACE: Duration = Duration::from_secs(20);

/// How long the modem has to have gone before the stand-in goes too.
const GONE: Duration = Duration::from_secs(60);

/// How often the stand-in looks.
const LOOK: Duration = Duration::from_secs(2);

/// Whether this program was started the way a host program starts VARA.
pub fn launched_as_vara(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter()
        .any(|arg| arg.eq_ignore_ascii_case("TCPIP"))
}

/// Be the stand-in if this program was started the way a host program starts VARA, until
/// the modem has gone; whether it was. `answers` says whether the modem's control port does,
/// `restarting` whether the modem last said it was stopping to be started again.
pub fn stood_in(answers: fn() -> bool, restarting: fn() -> bool) -> bool {
    if !launched_as_vara(std::env::args().skip(1)) {
        return false;
    }
    run(&mut Real {
        answers,
        restarting,
    });
    true
}

/// What the stand-in needs of the world, so its rules can be tested without one.
pub trait World {
    /// Whether the modem's control port answers.
    fn modem_answers(&mut self) -> bool;
    /// Whether the modem last said it was stopping to be started again.
    fn modem_restarting(&mut self) -> bool;
    /// Start Aether as the operator would, as a process of its own.
    fn start_aether(&mut self);
    /// Wait a while.
    fn wait(&mut self, how_long: Duration);
    /// The time now.
    fn now(&mut self) -> Instant;
}

/// Run the stand-in until the modem has been gone for [`GONE`].
pub fn run(world: &mut impl World) {
    let began = world.now();
    let restarting = !world.modem_answers() && world.modem_restarting();
    while !world.modem_answers() {
        if !restarting || world.now().duration_since(began) >= START_GRACE {
            world.start_aether();
            break;
        }
        world.wait(LOOK);
    }
    let mut gone_since: Option<Instant> = None;
    loop {
        world.wait(LOOK);
        if world.modem_answers() {
            gone_since = None;
            continue;
        }
        let now = world.now();
        if now.duration_since(*gone_since.get_or_insert(now)) >= GONE {
            return;
        }
    }
}

/// The real world: the control port, this program, the clock.
pub struct Real {
    /// Whether the modem's control port answers.
    pub answers: fn() -> bool,
    /// Whether the modem last said it was stopping to be started again.
    pub restarting: fn() -> bool,
}

impl World for Real {
    fn modem_answers(&mut self) -> bool {
        (self.answers)()
    }

    fn modem_restarting(&mut self) -> bool {
        (self.restarting)()
    }

    fn start_aether(&mut self) {
        let Ok(program) = std::env::current_exe() else {
            return;
        };
        let mut command = std::process::Command::new(program);
        // out of any job the host program runs its modem in, so that ending the stand-in
        // ends nothing of Aether; a job that refuses that leaves the plain start
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
            command.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
            if command.spawn().is_ok() {
                return;
            }
            command.creation_flags(0);
        }
        let _ = command.spawn();
    }

    fn wait(&mut self, how_long: Duration) {
        std::thread::sleep(how_long);
    }

    fn now(&mut self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world on a clock of its own: the modem answers from `up` to `down` (seconds after the
    /// start) but not during `blip`, and starting Aether brings it up after three seconds.
    struct Script {
        t0: Instant,
        elapsed: Duration,
        up: Option<f64>,
        down: Option<f64>,
        blip: Option<(f64, f64)>,
        restarting: bool,
        starts: Vec<f64>,
    }

    impl Script {
        fn new(up: Option<f64>, down: Option<f64>) -> Self {
            Self {
                t0: Instant::now(),
                elapsed: Duration::ZERO,
                up,
                down,
                blip: None,
                restarting: false,
                starts: Vec::new(),
            }
        }

        fn at(&self) -> f64 {
            self.elapsed.as_secs_f64()
        }
    }

    impl World for Script {
        fn modem_answers(&mut self) -> bool {
            let t = self.at();
            self.up.is_some_and(|up| t >= up)
                && self.down.is_none_or(|down| t < down)
                && self.blip.is_none_or(|(from, to)| !(from..to).contains(&t))
        }

        fn modem_restarting(&mut self) -> bool {
            self.restarting
        }

        fn start_aether(&mut self) {
            self.starts.push(self.at());
            self.up = Some(self.at() + 3.0);
        }

        fn wait(&mut self, how_long: Duration) {
            self.elapsed += how_long;
            assert!(self.at() < 3600.0, "the stand-in never left");
        }

        fn now(&mut self) -> Instant {
            self.t0 + self.elapsed
        }
    }

    #[test]
    fn trimodes_launch_is_told_from_the_operators() {
        let args = |line: &str| {
            line.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert!(launched_as_vara(args("TCPIP 127.0.0.1 8700")));
        assert!(launched_as_vara(args("tcpip 127.0.0.1 8300")));
        assert!(!launched_as_vara(args("")));
        assert!(!launched_as_vara(args("--from-the-menu")));
    }

    #[test]
    fn a_running_modem_is_left_alone_and_watched_until_it_goes() {
        let mut world = Script::new(Some(0.0), Some(300.0));
        run(&mut world);
        assert!(
            world.starts.is_empty(),
            "started Aether beside a running one"
        );
        // gone at 300 s, the stand-in goes a minute later, as VARA's process would
        assert!(
            (360.0..=364.0).contains(&world.at()),
            "left at {}",
            world.at()
        );
    }

    #[test]
    fn a_modem_between_a_stop_and_a_start_is_waited_for() {
        // a profile switch: back after thirteen seconds, and nothing else started meanwhile
        let mut world = Script::new(Some(13.0), Some(200.0));
        world.restarting = true;
        run(&mut world);
        assert!(world.starts.is_empty(), "{:?}", world.starts);
    }

    #[test]
    fn a_restart_that_never_comes_starts_aether_after_the_grace() {
        let mut world = Script::new(None, Some(500.0));
        world.restarting = true;
        run(&mut world);
        assert_eq!(world.starts.len(), 1);
        assert!(
            (20.0..=22.0).contains(&world.starts[0]),
            "{:?}",
            world.starts
        );
    }

    #[test]
    fn no_modem_at_all_starts_aether_at_once() {
        // a cold start: the host program connects three seconds after it starts the stand-in
        let mut world = Script::new(None, Some(500.0));
        run(&mut world);
        assert_eq!(world.starts, vec![0.0]);
    }

    #[test]
    fn a_modem_back_within_the_minute_keeps_the_stand_in() {
        // down for forty seconds and back: still the host program's modem, not one to start
        let mut world = Script::new(Some(0.0), Some(400.0));
        world.blip = Some((100.0, 140.0));
        run(&mut world);
        assert!(world.starts.is_empty(), "{:?}", world.starts);
        assert!(world.at() >= 460.0, "left at {}", world.at());
    }
}
