//! The server's log: lines on its standard error, each with its time and its kind.
//!
//! `info` is the server's own course and `warn` is something that went wrong and that it got past. `debug`
//! is a line for every request, written only where the settings ask for it ([`config::Log`]).
//!
//! [`config::Log`]: crate::config::Log

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::ids;

fn write(kind: &str, line: &str) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    eprintln!("{} {kind:<5} {line}", ids::rfc3339(now));
}

/// Says what the server did of its own accord: that it listens, that it stopped.
pub fn info(line: &str) {
    write("info", line);
}

/// Says what went wrong, where the server goes on.
pub fn warn(line: &str) {
    write("warn", line);
}

/// Writes a line for the request once it is answered: its method, its route, the answer's status and
/// how long that took. The route is written as the server lays it out, with a name where an id, a link's
/// secret or a file's name stands in the address, and nothing is said of who asked.
pub async fn request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let route = request.extensions().get::<MatchedPath>().map(|route| route.as_str().to_owned());
    let began = Instant::now();
    let response = next.run(request).await;
    let (route, status) = (route.as_deref().unwrap_or("(no route)"), response.status().as_u16());
    write("debug", &format!("{method} {route} {status} {} ms", began.elapsed().as_millis()));
    response
}
