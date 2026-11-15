//! A document's socket: the line between one device and the document's room.
//!
//! The socket carries frames and knows nothing of what is in them: `letters-sync` writes and reads them
//! on both ends. Who may open it is settled before the socket is, so that a refusal is an answer with a
//! status, which a device can tell from a network that failed.

use std::time::Duration;

use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::Shared;
use crate::error::{Answer, Refusal};
use crate::rooms::{Out, Seat};
use crate::session::{Carrier, Signed, same_origin};

/// How long a write waits on a device that has stopped reading.
const STALLED: Duration = Duration::from_secs(30);

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/v1/documents/{id}/sync", get(open))
}

async fn open(
    State(app): State<Shared>,
    who: Signed,
    Path(id): Path<String>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Answer<Response> {
    // A browser sends its cookie along with a socket that any site's page opens, and no rule of the
    // browser keeps that page from reading what comes back.
    if who.carrier == Carrier::Cookie && !same_origin(&app, &headers) {
        return Err(Refusal::forbidden("This request did not come from the server's own pages."));
    }
    let (seat, out) = app.join(&who, &id).await?;
    let lasts = Duration::from_secs(who.ends.saturating_sub(app.now()));
    let limit = letters_sync::MAX_FRAME;
    let upgrade = upgrade.max_message_size(limit).max_frame_size(limit);
    Ok(upgrade.on_upgrade(move |socket| carry(socket, seat, out, lasts)))
}

/// Carries frames between a device and its seat until either side is done.
async fn carry(mut socket: WebSocket, seat: Seat, mut out: Receiver<Out>, lasts: Duration) {
    let over = tokio::time::sleep(lasts);
    tokio::pin!(over);
    let mut told = false;
    let closed = loop {
        tokio::select! {
            heard = socket.recv() => match heard {
                Some(Ok(Message::Binary(frame))) => seat.received(frame.into()).await,
                // The socket answers a ping by itself, and the room is spoken to in binary alone.
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Text(_))) => {}
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break None,
            },
            next = out.recv() => match next {
                Some(Out::Frame(frame)) => {
                    let sent = timeout(STALLED, socket.send(Message::Binary(frame.into()))).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        break None;
                    }
                }
                // The room said why in the frame before.
                Some(Out::Close) => break Some(close_code::NORMAL),
                // The room let the device go without a word: it fell too far behind, or the room is gone.
                None => break Some(close_code::AGAIN),
            },
            () = &mut over, if !told => {
                told = true;
                seat.signed_out().await;
            }
        }
    };
    if let Some(code) = closed {
        let close = Message::Close(Some(CloseFrame { code, reason: "".into() }));
        let _ = timeout(STALLED, socket.send(close)).await;
    }
    seat.leave().await;
}
