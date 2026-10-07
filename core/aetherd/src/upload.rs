//! Sending the shared zip to the station that asked for it, at the push of a button.
//!
//! `share.prepare` writes the zip (`share.rs`); until now the operator then had to attach it to
//! an email, and testers forgot to — and a recording is uncompressed 48 kHz audio, 5.8 MB a
//! minute, so a Test session's zip is past what an email carries anyway. The station that asks
//! for the files now puts in its request link where they go: the address of a small script in
//! its own Google account (`tools/drive_upload/`) and a code it issued for this request. The
//! daemon asks the script for a place to put one file — `begin` — and the script answers with a
//! Google Drive *resumable upload* session made with its own account's authority, scoped to that
//! one file in its folder; the daemon sends the zip there in pieces, picking up where a dropped
//! connection left off; and tells the script it is done — `finish` — which emails the station
//! that asked a link to it. Nothing secret is in this program: the script's address is no use
//! without a code, and a code is good for one sender, a size and a number of uploads.
//!
//! The work is on a thread of its own: an upload of tens of megabytes over a slow uplink takes
//! minutes, and the run loop must not stand still for it (`devices.rs` for the same lesson).
//! `status.upload` says how far it has got.
//!
//! The protocol is Google's published resumable upload (Drive API v3, "Perform a resumable
//! upload"): `PUT` the bytes with `Content-Range: bytes a-b/total`; 308 *Resume Incomplete* with
//! `Range: bytes=0-n` says what arrived; 200 or 201 with the file's metadata ends it; and after a
//! failure, a `PUT` with `Content-Range: bytes */total` and no body asks where to carry on.

use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};

/// The Aether project's upload script (`tools/drive_upload/`): where debug mode sends the
/// sessions a host program ran, and what the panel's *Send my files to the Aether project*
/// uses (`PROJECT_UPLOAD_URL` in `app/ui/app.js`, held to this by a test). Public: the script
/// bounds what it takes a day.
pub const PROJECT_ENDPOINT: &str = "https://script.google.com/macros/s/AKfycbw5KhON9utE7jinNxv4g0ndw3k9wyw9miUGAZwPqLhoqh2tNZDBst8CiewQc5Q87avCIg/exec";

/// The piece sent at a time: a multiple of 256 KiB, as the resumable protocol requires of every
/// piece but the last. Small enough that a dropped connection costs little.
pub const CHUNK: usize = 8 * 1024 * 1024;

/// The largest zip sent: a day of recordings with their audio is well under it.
pub const MAX_BYTES: u64 = 400 * 1024 * 1024;

/// Failures in a row before the upload is given up. A success resets the count.
pub const ATTEMPTS: u32 = 6;

/// Where uploads may go: a Google Apps Script web app, the only kind of address the panel's
/// request links carry. A link could name anything, and this program does not send an
/// operator's files to an address of a stranger's choosing that is not even that.
const SCRIPT_PREFIXES: [&str; 2] = [
    "https://script.google.com/macros/s/",
    "https://script.google.com/a/macros/",
];

/// Whether an upload may go to `endpoint`: a script web app's `/exec` address.
#[must_use]
pub fn endpoint_allowed(endpoint: &str) -> bool {
    SCRIPT_PREFIXES.iter().any(|p| endpoint.starts_with(p))
        && endpoint.ends_with("/exec")
        && !endpoint.contains(['?', '#', ' '])
}

/// Whether `code` looks like one the script issues: letters, digits, `-` and `_`.
#[must_use]
pub fn code_allowed(code: &str) -> bool {
    (4..=64).contains(&code.len())
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// An HTTP answer, as much of it as the upload reads.
#[derive(Debug, Clone, Default)]
pub struct Reply {
    /// The status code.
    pub status: u16,
    /// The `Location` header: where a script's answer is to be fetched.
    pub location: Option<String>,
    /// The `Range` header: how much of the file the upload session holds.
    pub range: Option<String>,
    /// The body, as text.
    pub body: String,
}

/// What the upload needs of HTTP, so the tests can stand in for Google.
pub trait Http: Send {
    /// `POST` a JSON text to the script.
    ///
    /// # Errors
    /// The connection's failure, in a sentence.
    fn post(&self, url: &str, body: &str) -> Result<Reply, String>;
    /// `GET` the script's answer where its redirect points.
    ///
    /// # Errors
    /// The connection's failure, in a sentence.
    fn get(&self, url: &str) -> Result<Reply, String>;
    /// `PUT` a piece of the file to the upload session.
    ///
    /// # Errors
    /// The connection's failure, in a sentence.
    fn put(&self, url: &str, content_range: &str, body: &[u8]) -> Result<Reply, String>;
}

/// HTTP over the network: `ureq` with rustls, through the proxy the environment names.
pub struct Web {
    agent: ureq::Agent,
}

impl Default for Web {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            // a 308 is the upload session's "carry on", and the script answers by redirect:
            // both are read here, not followed or taken for errors
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent(concat!("aether-hf/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent }
    }
}

impl Web {
    fn reply(
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Reply, String> {
        let mut response = response.map_err(|error| error.to_string())?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let location = header("location");
        let range = header("range");
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string().unwrap_or_default();
        Ok(Reply {
            status,
            location,
            range,
            body,
        })
    }
}

impl Http for Web {
    fn post(&self, url: &str, body: &str) -> Result<Reply, String> {
        // text/plain: what a script web app reads as `e.postData.contents` without fuss
        Self::reply(
            self.agent
                .post(url)
                .header("Content-Type", "text/plain; charset=utf-8")
                .send(body),
        )
    }

    fn get(&self, url: &str) -> Result<Reply, String> {
        Self::reply(self.agent.get(url).call())
    }

    fn put(&self, url: &str, content_range: &str, body: &[u8]) -> Result<Reply, String> {
        Self::reply(
            self.agent
                .put(url)
                .header("Content-Range", content_range)
                .send(body),
        )
    }
}

/// One upload asked for.
#[derive(Debug, Clone)]
pub struct Job {
    /// The zip.
    pub file: PathBuf,
    /// Its name, as the station that asked will see it.
    pub name: String,
    /// The script's address.
    pub endpoint: String,
    /// The code the station that asked issued.
    pub code: String,
    /// What the sender wants to say with it.
    pub note: String,
    /// The sending station's callsign.
    pub callsign: String,
    /// The zip's contents, when it is still to be written: debug mode writes it on the
    /// upload's thread, not the run loop's — a session's audio is tens of megabytes, and the
    /// modem must not stand still while it is copied (ADR-0050). The time is the zip's.
    pub zip: Option<(Vec<(String, crate::share::Entry)>, u64)>,
}

/// How an upload is going, as `status.upload` reports it.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Progress {
    /// `idle`, `starting`, `sending`, `finishing`, `done` or `failed`.
    pub state: &'static str,
    /// The zip's name.
    pub name: Option<String>,
    /// Bytes the upload session holds.
    pub sent: u64,
    /// The zip's size.
    pub total: u64,
    /// Why it failed, in a sentence.
    pub error: Option<String>,
    /// Where the file is now, as the script says.
    pub link: Option<String>,
    /// Who it went to, as the script names the station that asked.
    pub to: Option<String>,
}

/// The upload under way or last made. Clones share it.
#[derive(Debug, Clone, Default)]
pub struct Upload {
    shared: Arc<Mutex<Progress>>,
}

impl Upload {
    /// How it is going.
    #[must_use]
    pub fn progress(&self) -> Progress {
        let progress = self.shared.lock().map(|p| p.clone()).unwrap_or_default();
        if progress.state.is_empty() {
            Progress {
                state: "idle",
                ..progress
            }
        } else {
            progress
        }
    }

    /// Whether one is under way.
    #[must_use]
    pub fn busy(&self) -> bool {
        matches!(self.progress().state, "starting" | "sending" | "finishing")
    }

    /// Begin `job` on a thread of its own, over `http`.
    ///
    /// # Errors
    /// Another upload is under way, or the thread would not start.
    pub fn start(&self, job: Job, http: Box<dyn Http>) -> Result<(), String> {
        {
            let Ok(mut progress) = self.shared.lock() else {
                return Err("The upload's state is unreadable.".into());
            };
            if matches!(progress.state, "starting" | "sending" | "finishing") {
                return Err("Another upload is under way.".into());
            }
            *progress = Progress {
                state: "starting",
                name: Some(job.name.clone()),
                ..Progress::default()
            };
        }
        let shared = Arc::clone(&self.shared);
        std::thread::Builder::new()
            .name("upload".into())
            .spawn(move || {
                let outcome = write(&job)
                    .and_then(|()| run(http.as_ref(), &job, &shared, Duration::from_secs(2)));
                if let Ok(mut progress) = shared.lock() {
                    match outcome {
                        Ok((link, to)) => {
                            progress.state = "done";
                            progress.sent = progress.total;
                            progress.link = link;
                            progress.to = to;
                        }
                        Err(error) => {
                            progress.state = "failed";
                            progress.error = Some(error);
                        }
                    }
                }
            })
            .map(|_| ())
            .map_err(|error| format!("The upload could not start: {error}"))
    }
}

/// Write the job's zip, when it carries one to write, beside the others in `shared/`.
///
/// # Errors
/// The zip could not be written.
fn write(job: &Job) -> Result<(), String> {
    let Some((entries, now_ms)) = &job.zip else {
        return Ok(());
    };
    if let Some(dir) = job.file.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("Could not write {}: {error}", job.name))?;
    }
    crate::share::write_zip(&job.file, entries, *now_ms)
        .map_err(|error| format!("Could not write {}: {error}", job.name))?;
    if let Some(dir) = job.file.parent() {
        crate::share::prune(dir, crate::share::KEEP);
    }
    Ok(())
}

/// Ask the script something: a `POST` of `payload`, its answer fetched where the redirect
/// points (a script web app answers that way), read as JSON.
///
/// # Errors
/// The connection's failure, an answer that is not the script's, or the script's refusal.
fn ask(http: &dyn Http, endpoint: &str, payload: &Value) -> Result<Value, String> {
    let mut reply = http.post(endpoint, &payload.to_string())?;
    if (300..400).contains(&reply.status) {
        let Some(location) = reply.location.clone() else {
            return Err(format!(
                "The upload script answered {} without an address.",
                reply.status
            ));
        };
        reply = http.get(&location)?;
    }
    if reply.status != 200 {
        return Err(format!(
            "The upload script answered {}: is the address in the request link right?",
            reply.status
        ));
    }
    let answer: Value = serde_json::from_str(&reply.body).map_err(|_| {
        "The upload address did not answer as the upload script does: is the request link \
         whole?"
            .to_owned()
    })?;
    if answer.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(answer)
    } else {
        Err(answer
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("The upload script refused, and did not say why.")
            .to_owned())
    }
}

/// The byte to carry on from, from a 308's `Range: bytes=0-n` (none: nothing arrived).
fn next_from(range: Option<&str>) -> u64 {
    range
        .and_then(|r| r.rsplit('-').next())
        .and_then(|n| n.trim().parse::<u64>().ok())
        .map_or(0, |last| last + 1)
}

/// The pieces, to the upload session at `session`, carrying on after failures: the id Google
/// gave the file.
///
/// # Errors
/// What stopped it, in a sentence the panel shows.
fn send_pieces(
    http: &dyn Http,
    session: &str,
    file: &mut File,
    total: u64,
    name: &str,
    shared: &Mutex<Progress>,
    backoff: Duration,
) -> Result<String, String> {
    let mut offset = 0u64;
    let mut failures = 0u32;
    let mut file_id: Option<String> = None;
    let mut buffer = vec![0u8; CHUNK];
    while file_id.is_none() {
        let attempt = if offset < total {
            let len = usize::try_from((total - offset).min(CHUNK as u64)).unwrap_or(CHUNK);
            file.seek(SeekFrom::Start(offset))
                .and_then(|_| file.read_exact(&mut buffer[..len]))
                .map_err(|error| format!("Cannot read {name}: {error}"))?;
            let range = format!("bytes {offset}-{}/{total}", offset + len as u64 - 1);
            http.put(session, &range, &buffer[..len])
        } else {
            // everything sent and nothing to say so: ask the session where it stands
            http.put(session, &format!("bytes */{total}"), &[])
        };
        match attempt {
            Ok(reply) if matches!(reply.status, 200 | 201) => {
                let meta: Value = serde_json::from_str(&reply.body).unwrap_or(Value::Null);
                file_id = Some(
                    meta.get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                );
                offset = total;
                failures = 0;
            }
            Ok(reply) if reply.status == 308 => {
                offset = next_from(reply.range.as_deref());
                failures = 0;
            }
            Ok(reply) if matches!(reply.status, 404 | 410) => {
                return Err(
                    "The place the upload script gave has expired; send again to start over."
                        .into(),
                );
            }
            other => {
                failures += 1;
                if failures > ATTEMPTS {
                    let why = match other {
                        Ok(reply) => format!("Google answered {}", reply.status),
                        Err(error) => error,
                    };
                    return Err(format!(
                        "The upload stopped after {ATTEMPTS} tries at {offset} of {total} bytes: \
                         {why}."
                    ));
                }
                std::thread::sleep(backoff * failures);
                // where does the session stand? A piece may have arrived in part
                if let Ok(reply) = http.put(session, &format!("bytes */{total}"), &[]) {
                    match reply.status {
                        308 => offset = next_from(reply.range.as_deref()),
                        200 | 201 => offset = total,
                        _ => {}
                    }
                }
            }
        }
        if let Ok(mut progress) = shared.lock() {
            progress.sent = offset;
        }
    }
    Ok(file_id.unwrap_or_default())
}

/// The upload: `begin`, the pieces, `finish`. Returns the link and who it went to.
///
/// # Errors
/// What stopped it, in a sentence the panel shows.
pub fn run(
    http: &dyn Http,
    job: &Job,
    shared: &Mutex<Progress>,
    backoff: Duration,
) -> Result<(Option<String>, Option<String>), String> {
    let set = |f: &dyn Fn(&mut Progress)| {
        if let Ok(mut progress) = shared.lock() {
            f(&mut progress);
        }
    };
    let mut file =
        File::open(&job.file).map_err(|error| format!("Cannot read {}: {error}", job.name))?;
    let total = file
        .metadata()
        .map_err(|error| format!("Cannot read {}: {error}", job.name))?
        .len();
    if total == 0 || total > MAX_BYTES {
        return Err(format!(
            "{} is {total} bytes: an upload is 1 byte to {} MB.",
            job.name,
            MAX_BYTES / 1024 / 1024
        ));
    }
    set(&|p| p.total = total);
    let begun = ask(
        http,
        &job.endpoint,
        &json!({
            "action": "begin",
            "code": job.code,
            "name": job.name,
            "size": total,
            "callsign": job.callsign,
            "version": env!("CARGO_PKG_VERSION"),
        }),
    )?;
    let Some(session) = begun.get("upload_url").and_then(Value::as_str) else {
        return Err("The upload script gave no place to put the file.".into());
    };
    let to = begun.get("to").and_then(Value::as_str).map(str::to_owned);
    set(&|p| p.state = "sending");

    let file_id = send_pieces(http, session, &mut file, total, &job.name, shared, backoff)?;
    set(&|p| p.state = "finishing");
    let finished = ask(
        http,
        &job.endpoint,
        &json!({
            "action": "finish",
            "code": job.code,
            "file_id": file_id,
            "name": job.name,
            "size": total,
            "callsign": job.callsign,
            "note": job.note,
        }),
    )?;
    let link = finished
        .get("link")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok((link, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_script_web_app_is_an_upload_address() {
        assert!(endpoint_allowed(
            "https://script.google.com/macros/s/AKfycbx123/exec"
        ));
        assert!(endpoint_allowed(
            "https://script.google.com/a/macros/example.org/s/AKfy/exec"
        ));
        for refused in [
            "http://script.google.com/macros/s/AKfy/exec",
            "https://script.google.com.evil.example/macros/s/x/exec",
            "https://evil.example/macros/s/x/exec",
            "https://script.google.com/macros/s/AKfy/dev",
            "https://script.google.com/macros/s/AKfy/exec?x=1",
        ] {
            assert!(!endpoint_allowed(refused), "{refused}");
        }
        assert!(code_allowed("WC4Y-7f3k9q"));
        assert!(!code_allowed("abc"));
        assert!(!code_allowed("has space"));
    }

    #[test]
    fn the_range_says_where_to_carry_on() {
        assert_eq!(next_from(Some("bytes=0-8388607")), 8_388_608);
        assert_eq!(next_from(None), 0);
    }

    /// Google, as far as the upload sees it: the script's two answers by redirect, and an
    /// upload session that keeps what arrives — and drops the connection once, partway.
    struct Fake {
        received: Mutex<Vec<u8>>,
        puts: Mutex<u32>,
        drop_at: u32,
        refuse_code: bool,
        finished: Mutex<Option<Value>>,
        answer: Mutex<String>,
    }

    impl Fake {
        fn new(drop_at: u32) -> Self {
            Self {
                received: Mutex::new(Vec::new()),
                puts: Mutex::new(0),
                drop_at,
                refuse_code: false,
                finished: Mutex::new(None),
                answer: Mutex::new(String::new()),
            }
        }
    }

    impl Http for Fake {
        fn post(&self, url: &str, body: &str) -> Result<Reply, String> {
            assert_eq!(url, "https://script.google.com/macros/s/AKfy/exec");
            let asked: Value = serde_json::from_str(body).expect("json");
            let answer = match asked["action"].as_str() {
                Some("begin") if self.refuse_code => {
                    json!({"ok": false, "error": "This code is not known."})
                }
                Some("begin") => {
                    assert_eq!(asked["code"], "WC4Y-abc123");
                    json!({"ok": true, "upload_url": "https://www.googleapis.com/upload/x", "to": "KK4ODA"})
                }
                Some("finish") => {
                    *self.finished.lock().unwrap() = Some(asked.clone());
                    json!({"ok": true, "link": "https://drive.google.com/file/d/F1/view"})
                }
                _ => json!({"ok": false, "error": "?"}),
            };
            *self.answer.lock().unwrap() = answer.to_string();
            Ok(Reply {
                status: 302,
                location: Some("https://script.googleusercontent.com/echo".into()),
                ..Reply::default()
            })
        }

        fn get(&self, url: &str) -> Result<Reply, String> {
            assert_eq!(url, "https://script.googleusercontent.com/echo");
            Ok(Reply {
                status: 200,
                body: self.answer.lock().unwrap().clone(),
                ..Reply::default()
            })
        }

        fn put(&self, url: &str, content_range: &str, body: &[u8]) -> Result<Reply, String> {
            assert_eq!(url, "https://www.googleapis.com/upload/x");
            *self.puts.lock().unwrap() += 1;
            let total: usize = content_range.rsplit('/').next().unwrap().parse().unwrap();
            let held = |received: &Vec<u8>| Reply {
                status: 308,
                range: (!received.is_empty()).then(|| format!("bytes=0-{}", received.len() - 1)),
                ..Reply::default()
            };
            if content_range.starts_with("bytes */") {
                return Ok(held(&self.received.lock().unwrap()));
            }
            let start: usize = content_range
                .trim_start_matches("bytes ")
                .split('-')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(
                start,
                self.received.lock().unwrap().len(),
                "carried on from what arrived"
            );
            if *self.puts.lock().unwrap() == self.drop_at {
                // half the piece arrives, then the connection drops
                self.received
                    .lock()
                    .unwrap()
                    .extend_from_slice(&body[..body.len() / 2]);
                return Err("connection reset".into());
            }
            self.received.lock().unwrap().extend_from_slice(body);
            if self.received.lock().unwrap().len() == total {
                Ok(Reply {
                    status: 200,
                    body: json!({"id": "F1"}).to_string(),
                    ..Reply::default()
                })
            } else {
                Ok(held(&self.received.lock().unwrap()))
            }
        }
    }

    fn zip_of(len: usize) -> (PathBuf, Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("aether-upload-{}-{len}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("aether-W4TGA-test.zip");
        let bytes: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        (path, bytes)
    }

    fn job(path: PathBuf) -> Job {
        Job {
            file: path,
            name: "aether-W4TGA-test.zip".into(),
            endpoint: "https://script.google.com/macros/s/AKfy/exec".into(),
            code: "WC4Y-abc123".into(),
            note: "the Test of last night".into(),
            callsign: "W4TGA".into(),
            zip: None,
        }
    }

    #[test]
    fn a_zip_goes_in_pieces_and_carries_on_after_a_dropped_connection() {
        // two and a half pieces, the second dropped halfway: the session is asked where it
        // stands and the rest goes from there, every byte once
        let (path, bytes) = zip_of(CHUNK * 2 + CHUNK / 2);
        let fake = Fake::new(2);
        let shared = Mutex::new(Progress::default());
        let (link, to) = run(&fake, &job(path), &shared, Duration::ZERO).expect("sent");
        assert_eq!(*fake.received.lock().unwrap(), bytes);
        assert_eq!(
            link.as_deref(),
            Some("https://drive.google.com/file/d/F1/view")
        );
        assert_eq!(to.as_deref(), Some("KK4ODA"));
        let finished = fake.finished.lock().unwrap().clone().expect("finished");
        assert_eq!(finished["file_id"], "F1");
        assert_eq!(finished["note"], "the Test of last night");
        assert_eq!(shared.lock().unwrap().sent, bytes.len() as u64);
    }

    #[test]
    fn the_scripts_refusal_is_what_the_panel_shows() {
        let (path, _) = zip_of(1000);
        let mut fake = Fake::new(0);
        fake.refuse_code = true;
        let shared = Mutex::new(Progress::default());
        let error = run(&fake, &job(path), &shared, Duration::ZERO).expect_err("refused");
        assert_eq!(error, "This code is not known.");
        assert_eq!(*fake.puts.lock().unwrap(), 0);
    }

    #[test]
    fn an_upload_gives_up_after_its_tries() {
        // every piece dropped: the upload stops, saying where
        struct Dead;
        impl Http for Dead {
            fn post(&self, _: &str, body: &str) -> Result<Reply, String> {
                let asked: Value = serde_json::from_str(body).unwrap();
                assert_eq!(asked["action"], "begin");
                Ok(Reply {
                    status: 200,
                    body: json!({"ok": true, "upload_url": "u"}).to_string(),
                    ..Reply::default()
                })
            }
            fn get(&self, _: &str) -> Result<Reply, String> {
                unreachable!()
            }
            fn put(&self, _: &str, _: &str, _: &[u8]) -> Result<Reply, String> {
                Err("no route to host".into())
            }
        }
        let (path, _) = zip_of(1000);
        let shared = Mutex::new(Progress::default());
        let error = run(&Dead, &job(path), &shared, Duration::ZERO).expect_err("gave up");
        assert!(
            error.contains("after 6 tries at 0 of 1000 bytes"),
            "{error}"
        );
        assert!(error.contains("no route to host"), "{error}");
    }

    #[test]
    fn the_real_client_reads_redirects_and_resume_incomplete() {
        // `Web` against a plain HTTP server on loopback: the script's 302, the session's 308
        // with no Location, then 201 — none of them followed or taken for an error
        use std::io::{BufRead as _, BufReader, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..4 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut length = 0usize;
                let mut range = String::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let lower = header.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if let Some(v) = header
                        .strip_prefix("Content-Range:")
                        .or_else(|| header.strip_prefix("content-range:"))
                    {
                        range = v.trim().to_owned();
                    }
                    if header == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).unwrap();
                seen.push((line.trim().to_owned(), range, body.len()));
                let reply = if line.starts_with("POST /exec") {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/echo\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else if line.starts_with("GET /echo") {
                    let b = r#"{"ok":true,"upload_url":"x"}"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                        b.len()
                    )
                } else if seen.len() == 3 {
                    "HTTP/1.1 308 Resume Incomplete\r\nRange: bytes=0-9\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                } else {
                    let b = r#"{"id":"F9"}"#;
                    format!(
                        "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                        b.len()
                    )
                };
                let mut stream = stream;
                stream.write_all(reply.as_bytes()).unwrap();
            }
            seen
        });
        let web = Web::default();
        let base = format!("http://127.0.0.1:{port}");
        let answer = ask(&web, &format!("{base}/exec"), &json!({"action": "begin"})).expect("ok");
        assert_eq!(answer["upload_url"], "x");
        let held = web
            .put(&format!("{base}/up"), "bytes 0-9/20", &[1; 10])
            .expect("308");
        assert_eq!((held.status, next_from(held.range.as_deref())), (308, 10));
        let done = web
            .put(&format!("{base}/up"), "bytes 10-19/20", &[2; 10])
            .expect("201");
        assert_eq!(done.status, 201);
        let seen = server.join().unwrap();
        assert!(seen[0].0.starts_with("POST /exec"), "{seen:?}");
        assert!(seen[1].0.starts_with("GET /echo"), "{seen:?}");
        assert_eq!((seen[2].1.as_str(), seen[2].2), ("bytes 0-9/20", 10));
        assert_eq!((seen[3].1.as_str(), seen[3].2), ("bytes 10-19/20", 10));
    }
}
