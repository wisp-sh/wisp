//! A worker owns guest execution. Dropping or timing out the client kills it.
use crate::{MAX_FRAME, Request, Response, VERSION};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

pub struct Client {
    child: Child,
    requests: Option<mpsc::Sender<Vec<u8>>>,
    replies: mpsc::Receiver<io::Result<Response>>,
    reader: Option<thread::JoinHandle<()>>,
    next_id: u64,
    dead: bool,
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
impl Client {
    pub fn spawn(path: &Path, cache: Option<&Path>) -> io::Result<Self> {
        Self::spawn_with_options(path, cache, false)
    }
    /// Start a worker that may load trusted native Wasmtime artifacts.
    /// The caller must authenticate these artifacts before submitting their paths.
    /// A checksum supplied alongside an untrusted artifact does not establish trust.
    pub fn spawn_with_precompiled(path: &Path, cache: Option<&Path>) -> io::Result<Self> {
        Self::spawn_with_options(path, cache, true)
    }
    fn spawn_with_options(
        path: &Path,
        cache: Option<&Path>,
        allow_precompiled: bool,
    ) -> io::Result<Self> {
        let mut command = Command::new(path);
        command
            .arg("serve")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if allow_precompiled {
            command.arg("--allow-precompiled");
        }
        // Required for process startup on Windows; never inherited by guests.
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        if let Some(cache) = cache {
            command.env("HOME", cache).env("XDG_CACHE_HOME", cache);
        }
        let mut child = command.spawn()?;
        let mut input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        let (requests, incoming) = mpsc::channel::<Vec<u8>>();
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            for request in incoming {
                let reply = (|| {
                    input.write_all(&request)?;
                    input.write_all(b"\n")?;
                    input.flush()?;
                    let mut frame = Vec::new();
                    let n = (&mut output)
                        .take((MAX_FRAME + 1) as u64)
                        .read_until(b'\n', &mut frame)?;
                    if n == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "worker exited",
                        ));
                    }
                    if n > MAX_FRAME || frame.last() != Some(&b'\n') {
                        return Err(invalid("invalid worker frame"));
                    }
                    serde_json::from_slice(&frame).map_err(|e| invalid(e.to_string()))
                })();
                let failed = reply.is_err();
                if send.send(reply).is_err() || failed {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            requests: Some(requests),
            replies,
            reader: Some(reader),
            next_id: 1,
            dead: false,
        };
        let ping = client.request(Request {
            operation: "ping".into(),
            ..Request::default()
        })?;
        if !ping.ok {
            return Err(invalid("worker handshake failed"));
        }
        Ok(client)
    }
    pub fn request(&mut self, mut request: Request) -> io::Result<Response> {
        if self.dead {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "worker terminated",
            ));
        }
        request.id = self.next_id;
        self.next_id += 1;
        let result = (|| {
            let frame = serde_json::to_vec(&request).map_err(|e| invalid(e.to_string()))?;
            if frame.len() >= MAX_FRAME {
                return Err(invalid("worker request exceeds frame limit"));
            }
            self.requests.as_ref().unwrap().send(frame).map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "worker transport stopped")
            })?;
            let reply = self
                .replies
                .recv_timeout(Duration::from_millis(request.limits.timeout_ms))
                .map_err(|e| {
                    io::Error::new(io::ErrorKind::TimedOut, format!("worker response: {e}"))
                })??;
            if reply.version != VERSION || reply.id != request.id {
                return Err(invalid("worker protocol mismatch"));
            }
            Ok(reply)
        })();
        if result.is_err() {
            self.terminate();
        }
        result
    }
    pub fn terminate(&mut self) {
        self.dead = true;
        self.requests.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        // A compliant worker sends one response for each request. A malicious
        // executable is outside this protocol's trust boundary.
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.terminate();
    }
}
