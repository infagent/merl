//! A synthetic fixed-endpoint provider; no live model or corpus data.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct MockProvider {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockProvider {
    pub fn start(mode: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let stop = Arc::clone(&stop);
            let requests = Arc::clone(&requests);
            let mode = mode.to_owned();
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((mut stream, _)) = listener.accept() {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") {
                            stream.read_exact(&mut byte).unwrap();
                            request.push(byte[0]);
                            assert!(request.len() < 32_768);
                        }
                        let headers = String::from_utf8(request).unwrap();
                        let size: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap();
                        assert!(size <= 1_048_576);
                        let mut bytes = vec![0; size];
                        stream.read_exact(&mut bytes).unwrap();
                        let body: Value = serde_json::from_slice(&bytes).unwrap();
                        requests
                            .lock()
                            .unwrap()
                            .push(format!("{headers}{}", String::from_utf8(bytes).unwrap()));
                        if mode == "unavailable" {
                            continue;
                        }
                        let mut response = json!({"text":"Synthetic amber answer.","usage":{"input":10,"output":5},"model":"synthetic-resolved-v1","request_id":"mock-1"});
                        if mode == "package" {
                            let is_grader = headers.starts_with("POST /grader ");
                            let expected = if is_grader {
                                "grader-secret"
                            } else {
                                "reader-secret"
                            };
                            assert!(
                                headers
                                    .to_lowercase()
                                    .contains(&format!("authorization: bearer {expected}\r\n"))
                            );
                            let text = body["messages"][0]["text"].as_str().unwrap();
                            if is_grader {
                                assert!(text.contains("required_meanings"));
                            } else {
                                assert!(!text.contains("required_meanings"));
                                assert!(!text.contains("violet"));
                            }
                        }
                        let mut status = "200 OK";
                        let mut extra = String::new();
                        match mode.as_str() {
                            "redirect" => {
                                status = "307 Temporary Redirect";
                                extra = format!("Location: http://{address}/forbidden\r\n");
                            }
                            "provider_failure" => status = "429 Too Many Requests",
                            "missing_usage" => {
                                response.as_object_mut().unwrap().remove("usage");
                            }
                            _ => {}
                        }
                        let response = response_body(&mode, response);
                        write!(stream,"HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
                    } else {
                        thread::sleep(Duration::from_millis(2));
                    }
                }
            })
        };
        Self {
            port: address.port(),
            requests,
            stop,
            thread: Some(thread),
        }
    }
    pub fn endpoint(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port())
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for MockProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn response_body(mode: &str, mut response: Value) -> String {
    match mode {
        "credential_echo" | "credential_echo_escaped_text" => {
            response["text"] = json!("synthetic-secret-value");
        }
        "credential_echo_escaped_model" => {
            response["model"] = json!("synthetic-secret-value");
        }
        "credential_echo_escaped_nested_value" => {
            response["metadata"] = json!([{"nested":"prefix synthetic-secret-value suffix"}]);
        }
        "credential_echo_escaped_key" => {
            response["metadata"] = json!([{"synthetic-secret-value":"synthetic metadata"}]);
        }
        _ => {}
    }
    let response = response.to_string();
    if mode.starts_with("credential_echo_escaped_") {
        response.replace("synthetic-secret-value", r"\u0073ynthetic-secret-value")
    } else {
        response
    }
}
