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

use std::sync::mpsc;

use serde::{Deserialize, Serialize};

/// A request from a client.
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
    /// Bearer token, for the first message on a non-loopback connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
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
    /// What happened: `state`, `metrics`, `data`, `ptt`, `busy`, `log`.
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
}

/// The client end: a handle a connection uses to talk to the modem.
#[derive(Debug, Clone)]
pub struct ControlHandle {
    commands: mpsc::Sender<Command>,
    subscribers: Subscribers,
}

type Subscribers = std::sync::Arc<std::sync::Mutex<Vec<mpsc::Sender<Event>>>>;

/// Build both ends of a control channel.
#[must_use]
pub fn channel() -> (ControlHandle, ControlChannel) {
    let (sender, receiver) = mpsc::channel();
    let subscribers: Subscribers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    (
        ControlHandle {
            commands: sender,
            subscribers: std::sync::Arc::clone(&subscribers),
        },
        ControlChannel {
            commands: receiver,
            subscribers,
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
            token: None,
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
