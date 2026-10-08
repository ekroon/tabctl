use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct HttpFixture {
    pub url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    completed_pages: Arc<AtomicUsize>,
    diagnostics: Arc<Mutex<Vec<String>>>,
}

impl HttpFixture {
    pub fn html(body: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind HTTP fixture");
        listener
            .set_nonblocking(true)
            .expect("nonblocking HTTP listener");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let completed_pages = Arc::new(AtomicUsize::new(0));
        let completed = Arc::clone(&completed_pages);
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let report = Arc::clone(&diagnostics);
        let body = Arc::new(body.into_bytes());
        let worker = thread::spawn(move || {
            let mut connections = Vec::new();
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let body = Arc::clone(&body);
                        let completed = Arc::clone(&completed);
                        let report = Arc::clone(&report);
                        // macOS accepted sockets inherit the listener's nonblocking mode.
                        stream
                            .set_nonblocking(false)
                            .expect("blocking HTTP connection");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        connections.push(thread::spawn(move || {
                            let outcome = serve_connection(stream, &body, &completed);
                            report.lock().unwrap().push(outcome);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("HTTP fixture accept: {error}"),
                }
            }
            for connection in connections {
                connection.join().expect("HTTP connection worker");
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
            completed_pages,
            diagnostics,
        }
    }

    pub fn completed_pages(&self) -> usize {
        self.completed_pages.load(Ordering::Acquire)
    }

    pub fn diagnostics(&self) -> String {
        self.diagnostics.lock().unwrap().join("; ")
    }
}

fn serve_connection(mut stream: TcpStream, body: &[u8], completed: &AtomicUsize) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) => return "connection closed before HTTP headers".to_string(),
            Ok(count) => {
                request.extend_from_slice(&buffer[..count]);
                if request.len() > 8192 {
                    return "HTTP request headers exceeded fixture limit".to_string();
                }
            }
            Err(error) => return format!("HTTP header read failed: {error}"),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let request_line = request.lines().next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let (status, response_body) = match (method, path) {
        ("GET" | "HEAD", "/") => ("200 OK", body),
        ("GET" | "HEAD", "/favicon.ico") => ("204 No Content", &[][..]),
        _ => ("404 Not Found", &[][..]),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    );
    let result = stream.write_all(header.as_bytes()).and_then(|_| {
        if method == "HEAD" {
            Ok(())
        } else {
            stream.write_all(response_body)
        }
    });
    match result {
        Ok(()) => {
            if method == "GET" && path == "/" {
                completed.fetch_add(1, Ordering::Release);
            }
            format!(
                "{request_line}: {status}, {} UTF-8 bytes sent",
                response_body.len()
            )
        }
        Err(error) => format!("{request_line}: response write failed: {error}"),
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() && !thread::panicking() {
                panic!("HTTP fixture thread failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_not_blocked_by_an_idle_preconnect() {
        let body = "<title>Actual response</title>😀".repeat(10_000);
        let fixture = HttpFixture::html(body.clone());
        let address = fixture.url.strip_prefix("http://").unwrap();
        let _idle = TcpStream::connect(address).unwrap();
        thread::sleep(Duration::from_millis(50));
        let mut navigation = TcpStream::connect(address).unwrap();
        navigation
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        navigation
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = Vec::new();
        navigation
            .read_to_end(&mut response)
            .expect("idle Chrome preconnect must not block navigation");
        let response = String::from_utf8(response).unwrap();
        let (headers, received) = response
            .split_once("\r\n\r\n")
            .expect("complete HTTP response");
        assert!(headers.contains(&format!("Content-Length: {}", body.len())));
        assert_eq!(received, body);
    }

    #[test]
    fn large_unicode_body_is_delivered_completely_despite_delayed_reads() {
        let body = "\"\\😀".repeat(1_600_000);
        let fixture = HttpFixture::html(body.clone());
        let mut client = TcpStream::connect(fixture.url.strip_prefix("http://").unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        let response = String::from_utf8(response).unwrap();
        assert_eq!(response.split_once("\r\n\r\n").unwrap().1, body);
        assert_eq!(fixture.completed_pages(), 1);
    }
}
