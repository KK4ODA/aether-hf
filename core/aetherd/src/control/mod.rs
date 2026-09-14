//! The control API: `docs/spec/control-api.md`.
//!
//! * [`protocol`] — the wire messages, and the channel between a connection and the modem.
//! * [`methods`] — what each method does, as a function of a request and a station.
//! * [`server`] — the socket: WebSocket for a session, `POST /v1/<method>` for a one-shot.
//!
//! The split is deliberate. Everything above the socket can be tested by calling it, so the
//! claims the specification makes — that `disconnect` is orderly and `abort` is not, that a
//! client discovers the mode table rather than hard-coding it, that an unknown method is
//! refused — are checked in the test suite rather than by hand against a running daemon.

pub mod methods;
pub mod protocol;
pub mod server;

pub use protocol::{ApiError, ControlChannel, ControlHandle, Event, Request, Response, channel};
pub use server::{ControlConfig, ControlServer};
