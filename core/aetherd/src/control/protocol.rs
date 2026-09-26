//! The wire messages of the control API, and the channel between a client and the modem.
//!
//! This is `docs/spec/control-api.md` in types: requests correlated by `id`, one response
//! each, and unsolicited events. Everything here is pure data and pure plumbing — it can be
//! exercised without a socket, which is where the tests are.
//!
//! # Why a channel and not a lock
//!
//! The modem is single-threaded on purpose: it holds a decoder that frames borrow while the
//! link engine decides what to do with them, and its clock is the audio it has heard. A
//! control connection therefore cannot reach in and touch it. Instead each connection sends
//! [`Command`]s down a channel and gets a [`Reply`] back on one it supplied, which keeps the
//! modem's single-threadedness a fact rather than a convention.
//!
//! # Replies on their way when the daemon stops
//!
//! A reply is written by the connection's thread, after the modem has handed it over. A
//! daemon that stops — `shutdown`, with or without `restart`, is answered and then acted on
//! — must not exit in between, or the client that asked sees its connection close with no
//! answer: a restart that looks like a crash to the supervisor that asked for it. So a
//! connection counts its request as *unwritten* from the moment it asks until it has
//! written the reply ([`ControlHandle::call_and_deliver`]), and the modem's last act is to
//! refuse whatever is still asking and wait for that count to reach zero
//! ([`ControlChannel::settle`]).

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// A request from a client.
///
/// It carries no credential: a connection from another address presents the token once, as
/// `Authorization: Bearer` on its WebSocket upgrade or its `POST` (`server.rs`), before any
/// request is read. A `token` field a client sends anyway is ignored, as any field this
/// version does not know is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Correlates the response. A client may use any string.
    #[serde(default)]
    pub id: Option<String>,
    /// Which method to call.
    pub method: String,
    /// Method arguments.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A reply to one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The request's `id`, echoed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Whether the method succeeded.
    pub ok: bool,
    /// What it returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Why it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

/// A failure, as a client sees it.
///
/// `code` is the stable machine-readable field; `message` is written for the operator who
/// will read it, not for the developer who wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// Stable identifier, `snake_case`.
    pub code: String,
    /// What went wrong and what to do about it, in plain language.
    pub message: String,
    /// Whether trying again could work.
    pub retryable: bool,
}

impl ApiError {
    /// Build an error.
    #[must_use]
    pub fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            retryable,
        }
    }
}

impl Response {
    /// A successful reply.
    #[must_use]
    pub fn ok(id: Option<String>, result: serde_json::Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// A failed reply.
    #[must_use]
    pub fn failed(id: Option<String>, error: ApiError) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

/// Something that happened, sent to every subscriber without being asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// What happened: `state`, `metrics`, `frame`, `data`, `ptt`, `log`, … (`control-api.md` §5).
    pub event: String,
    /// The detail.
    pub data: serde_json::Value,
}

impl Event {
    /// Build an event.
    #[must_use]
    pub fn new(name: &str, data: serde_json::Value) -> Self {
        Self {
            event: name.to_owned(),
            data,
        }
    }
}

/// What a connection asks the modem to do.
///
/// Each carries the channel its reply goes back on, so a connection can have several requests
/// outstanding and the modem never has to remember who asked.
#[derive(Debug)]
pub struct Command {
    /// The request.
    pub request: Request,
    /// Where the reply goes.
    pub reply: mpsc::Sender<Response>,
}

/// The modem end of the channel: commands in, events out.
#[derive(Debug)]
pub struct ControlChannel {
    commands: mpsc::Receiver<Command>,
    subscribers: Subscribers,
    unwritten: Arc<AtomicUsize>,
}

/// The client end: a handle a connection uses to talk to the modem.
#[derive(Debug, Clone)]
pub struct ControlHandle {
    commands: mpsc::Sender<Command>,
    subscribers: Subscribers,
    unwritten: Arc<AtomicUsize>,
}

/// A request counted as unwritten for as long as this lives: from the moment a connection
/// asks until it has written the reply.
struct Unwritten<'a>(&'a AtomicUsize);

impl<'a> Unwritten<'a> {
    fn count(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for Unwritten<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

type Subscribers = std::sync::Arc<std::sync::Mutex<Vec<mpsc::Sender<Event>>>>;

/// Build both ends of a control channel.
#[must_use]
pub fn channel() -> (ControlHandle, ControlChannel) {
    let (sender, receiver) = mpsc::channel();
    let subscribers: Subscribers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let unwritten = Arc::new(AtomicUsize::new(0));
    (
        ControlHandle {
            commands: sender,
            subscribers: std::sync::Arc::clone(&subscribers),
            unwritten: Arc::clone(&unwritten),
        },
        ControlChannel {
            commands: receiver,
            subscribers,
            unwritten,
        },
    )
}

impl ControlHandle {
    /// Send a request and wait for its reply.
    ///
    /// # Errors
    /// If the modem has stopped.
    pub fn call(&self, request: Request) -> Result<Response, ApiError> {
        let (reply, replies) = mpsc::channel();
        self.commands
            .send(Command { request, reply })
            .map_err(|_| ApiError::new("modem_stopped", "The modem has stopped.", false))?;
        replies
            .recv()
            .map_err(|_| ApiError::new("modem_stopped", "The modem has stopped.", false))
    }

    /// [`call`](Self::call), and hand the reply — or the failure, as a reply — to `deliver`,
    /// the connection writing it to its client. The request counts as unwritten until
    /// `deliver` has returned, so a daemon stopping waits for it ([`ControlChannel::settle`]):
    /// the reply to the `shutdown` that stops it reaches the client that asked.
    pub fn call_and_deliver<T>(&self, request: Request, deliver: impl FnOnce(Response) -> T) -> T {
        let _unwritten = Unwritten::count(&self.unwritten);
        let response = self
            .call(request)
            .unwrap_or_else(|error| Response::failed(None, error));
        deliver(response)
    }

    /// Subscribe to events. The receiver stops when this handle's connection drops it.
    #[must_use]
    pub fn subscribe(&self) -> mpsc::Receiver<Event> {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }

    /// How many connections are listening for events.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().map_or(0, |list| list.len())
    }
}

impl ControlChannel {
    /// Take every request that has arrived, without waiting.
    #[must_use]
    pub fn drain(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }

    /// Send an event to every subscriber, dropping the ones whose connection has gone.
    pub fn publish(&self, event: &Event) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        subscribers.retain(|sender| sender.send(event.clone()).is_ok());
    }

    /// How many connections are listening.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().map_or(0, |list| list.len())
    }

    /// The modem's last act before the process exits: refuse whatever is still asking and
    /// wait, up to `limit`, until every reply on its way has been written by its connection
    /// ([`ControlHandle::call_and_deliver`]). Whether they all were.
    #[must_use]
    pub fn settle(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            for command in self.drain() {
                let _ = command.reply.send(Response::failed(
                    command.request.id.clone(),
                    ApiError::new("modem_stopped", "The modem is stopping.", false),
                ));
            }
            if self.unwritten.load(Ordering::SeqCst) == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(method: &str) -> Request {
        Request {
            id: Some("1".into()),
            method: method.to_owned(),
            params: json!({}),
        }
    }

    #[test]
    fn a_request_round_trips_through_the_shape_the_spec_documents() {
        let text = r#"{"id":"7","method":"connect","params":{"remote":"KK4XYZ"}}"#;
        let parsed: Request = serde_json::from_str(text).expect("parse");
        assert_eq!(parsed.id.as_deref(), Some("7"));
        assert_eq!(parsed.method, "connect");
        assert_eq!(parsed.params["remote"], "KK4XYZ");
    }

    #[test]
    fn a_token_in_a_request_is_not_how_a_connection_authenticates() {
        // the bearer token goes on the connection, never in a request: a request that carries
        // one is read as if it did not, and one serialised never says it had one
        let text = r#"{"id":"7","method":"status","token":"hunter2"}"#;
        let parsed: Request = serde_json::from_str(text).expect("parse");
        assert_eq!(parsed.method, "status");
        let again = serde_json::to_string(&parsed).expect("serialise");
        assert!(!again.contains("hunter2"), "{again}");
    }

    #[test]
    fn a_client_may_omit_the_id_and_the_params() {
        // a one-shot over REST has nothing to correlate, and `status` takes no arguments
        let parsed: Request = serde_json::from_str(r#"{"method":"status"}"#).expect("parse");
        assert_eq!(parsed.id, None);
        assert!(parsed.params.is_null() || parsed.params == json!({}));
    }

    #[test]
    fn a_response_carries_either_a_result_or_an_error_and_never_both() {
        let good = serde_json::to_value(Response::ok(Some("7".into()), json!({"session": 42})))
            .expect("serialise");
        assert_eq!(good["ok"], true);
        assert_eq!(good["result"]["session"], 42);
        assert!(good.get("error").is_none(), "a success carried an error");

        let bad = serde_json::to_value(Response::failed(
            Some("7".into()),
            ApiError::new("busy_channel", "The channel is busy.", true),
        ))
        .expect("serialise");
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["error"]["code"], "busy_channel");
        assert_eq!(bad["error"]["retryable"], true);
        assert!(bad.get("result").is_none(), "a failure carried a result");
    }

    #[test]
    fn an_event_has_no_id_so_a_client_cannot_mistake_it_for_a_reply() {
        let event = serde_json::to_value(Event::new("ptt", json!({"on": true}))).expect("value");
        assert!(event.get("id").is_none());
        assert_eq!(event["event"], "ptt");
        assert_eq!(event["data"]["on"], true);
    }

    #[test]
    fn a_command_reaches_the_modem_and_its_reply_comes_back() {
        let (handle, channel) = channel();
        let worker = std::thread::spawn(move || {
            // stand in for the modem's run loop: answer whatever arrives
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                let commands = channel.drain();
                let answered = !commands.is_empty();
                for command in commands {
                    let id = command.request.id.clone();
                    let _ = command
                        .reply
                        .send(Response::ok(id, json!({"state": "idle"})));
                }
                if answered {
                    return;
                }
                std::thread::yield_now();
            }
            panic!("the request never arrived");
        });
        let response = handle.call(request("status")).expect("call");
        assert!(response.ok);
        assert_eq!(response.result.expect("result")["state"], "idle");
        worker.join().expect("worker");
    }

    #[test]
    fn a_caller_is_told_when_the_modem_has_stopped_rather_than_waiting_for_ever() {
        let (handle, channel) = channel();
        drop(channel);
        let error = handle.call(request("status")).expect_err("it should fail");
        assert_eq!(error.code, "modem_stopped");
        assert!(!error.retryable);
    }

    #[test]
    fn a_reply_on_its_way_is_written_before_the_modem_stops() {
        // `shutdown` is answered and then acted on: without the wait, the process could exit
        // between the answer and the connection's write, and the client that asked — a
        // supervisor asking for a restart — saw its connection close with nothing on it
        let (handle, channel) = channel();
        let written = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&written);
        let connection = std::thread::spawn(move || {
            handle.call_and_deliver(request("shutdown"), |response| {
                // a connection slow to write: descheduled, or a loaded machine
                std::thread::sleep(Duration::from_millis(150));
                flag.store(true, Ordering::SeqCst);
                response
            })
        });
        let command = loop {
            if let Some(command) = channel.drain().pop() {
                break command;
            }
            std::thread::yield_now();
        };
        let _ = command.reply.send(Response::ok(
            command.request.id.clone(),
            json!({"stopping": true}),
        ));
        assert!(
            channel.settle(Duration::from_secs(5)),
            "a reply went unwritten"
        );
        assert!(
            written.load(Ordering::SeqCst),
            "the modem stopped before the reply was written"
        );
        assert!(connection.join().expect("connection").ok);
    }

    #[test]
    fn a_request_arriving_as_the_modem_stops_is_refused_not_left_waiting() {
        let (handle, channel) = channel();
        let connection = std::thread::spawn(move || {
            handle.call_and_deliver(request("status"), |response| response)
        });
        // once the connection has counted its request, the wait cannot end without it — the
        // request reaches the channel before or during the wait, and is answered either way
        while channel.unwritten.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        assert!(
            channel.settle(Duration::from_secs(5)),
            "a reply went unwritten"
        );
        let response = connection.join().expect("connection");
        assert!(!response.ok);
        assert_eq!(
            response.error.expect("error").code,
            "modem_stopped",
            "a request the modem will never answer is told so"
        );
    }

    #[test]
    fn with_nothing_on_its_way_the_modem_stops_at_once() {
        let (_handle, channel) = channel();
        let began = Instant::now();
        assert!(channel.settle(Duration::from_secs(5)));
        assert!(began.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn every_subscriber_gets_every_event() {
        let (handle, channel) = channel();
        let first = handle.subscribe();
        let second = handle.subscribe();
        assert_eq!(channel.subscriber_count(), 2);

        channel.publish(&Event::new("busy", json!({"busy": true})));
        for receiver in [&first, &second] {
            let event = receiver.try_recv().expect("event");
            assert_eq!(event.event, "busy");
        }
    }

    #[test]
    fn a_connection_that_has_gone_stops_being_sent_events() {
        // otherwise a client that closed its browser tab leaks a queue for the life of the
        // daemon, and every event pays for it
        let (handle, channel) = channel();
        let keep = handle.subscribe();
        drop(handle.subscribe());
        assert_eq!(channel.subscriber_count(), 2);

        channel.publish(&Event::new("log", json!({"message": "hello"})));
        assert_eq!(
            channel.subscriber_count(),
            1,
            "the dead subscriber was kept"
        );
        assert!(keep.try_recv().is_ok());
    }
}
