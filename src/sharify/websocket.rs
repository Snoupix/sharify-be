use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use actix::clock::interval;
use actix_web::{web, Error as ActixError, HttpRequest, HttpResponse};
use actix_ws::{AggregatedMessage, Session};

use crate::sharify::room::{RoomClientID, RoomID, RoomManager};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(HEARTBEAT_INTERVAL.as_secs() * 2);

#[derive(Clone)]
pub struct SharifyWsInstance {
    session: Session,
    room_id: RoomID,
    hb: Arc<Mutex<Instant>>,
}

impl std::fmt::Debug for SharifyWsInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharifyWsInstance")
            .field("room_id", &self.room_id)
            .finish_non_exhaustive()
    }
}

// TODO future: Make a ClientID map to a Vec<SharifyWsInstance> for 2 reasons:
// 1. The client can have multiple tabs open with the same session instead of overriding
// 2. The client could be on 2 different rooms (bigger feature)
#[derive(Default)]
pub struct SharifyWsManager {
    /// Maps a client_id to its Instance (ws_session, room_id, heartbeat)
    ws_sessions: HashMap<RoomClientID, SharifyWsInstance>,
}

impl SharifyWsInstance {
    fn new(room_id: RoomID, session: Session) -> Self {
        SharifyWsInstance {
            session,
            room_id,
            hb: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub async fn init(
        req: HttpRequest,
        body: web::Payload,
        sharify_ws_manager: web::Data<Arc<RwLock<SharifyWsManager>>>,
        sharify_state: web::Data<Arc<RwLock<RoomManager>>>,
        path: web::Path<(RoomID, RoomClientID)>,
    ) -> Result<HttpResponse, ActixError> {
        let (room_id, client_id) = path.into_inner();
        let state_guard = sharify_state.read().unwrap();
        let room = state_guard
            .get_room(&room_id)
            .ok_or(actix_web::error::ErrorBadRequest(format!(
                "Room {} does not exist",
                room_id
            )))?;

        let client = room.clients.iter().find(|e| e.id == client_id);

        if client.is_none() || room.banned_clients.contains(&client_id) {
            return Err(actix_web::error::ErrorUnauthorized(String::from(
                "You are not allowed to join this room",
            )));
        }

        if let Some(Self { session, .. }) = sharify_ws_manager
            .write()
            .unwrap()
            .ws_sessions
            .remove(&client_id)
        {
            let _ = session.close(None).await;
        }

        drop(state_guard);

        let mut sharify_guard = sharify_state.write().unwrap();
        if let Err(e) = sharify_guard.set_ws_client_state(room_id, &client_id, true) {
            return Err(actix_web::error::ErrorBadRequest(format!("WS Error: {e}")));
        }

        debug!(
            "WS Debug: Starting ws session for roomID {} and userID {}",
            room_id, client_id
        );

        let (res, mut session, stream) = actix_ws::handle(&req, body)?;
        let _self = Self::new(room_id, session.clone());

        // max 128kb stream
        let mut stream = stream.max_frame_size(1024 * 128).aggregate_continuations();

        sharify_ws_manager
            .write()
            .unwrap()
            .ws_sessions
            .insert(client_id.clone(), _self.clone());

        _self.init_heartbeat(Arc::clone(&sharify_ws_manager), client_id.clone());

        let hb = Arc::clone(&_self.hb);
        let sharify_state = Arc::clone(&sharify_state);
        let ws_manager = Arc::clone(&sharify_ws_manager);

        actix_web::rt::spawn(async move {
            while let Some(Ok(msg)) = stream.recv().await {
                match msg {
                    AggregatedMessage::Ping(bytes) => {
                        if session.pong(&bytes).await.is_err() {
                            break;
                        }
                    }
                    AggregatedMessage::Text(string) => {
                        info!("Relaying text, {string}");
                        let guard = sharify_state.read().unwrap();
                        let Some(room) = guard.get_room(&room_id) else {
                            continue;
                        };
                        let clients = room.clients.iter().map(|c| c.id.clone()).collect();

                        drop(guard);

                        _self
                            .send_in_room(Arc::clone(&ws_manager), clients, string)
                            .await;
                    }
                    AggregatedMessage::Close(_) => {
                        break;
                    }
                    AggregatedMessage::Pong(_) => {
                        *hb.lock().unwrap() = Instant::now();
                    }
                    AggregatedMessage::Binary(bytes) => {}
                };
            }

            // TODO: Remove from Room
            ws_manager.write().unwrap().ws_sessions.remove(&client_id);
            let _ = session.close(None).await;
        });

        Ok(res)
    }

    fn init_heartbeat(&self, ws_manager: Arc<RwLock<SharifyWsManager>>, client_id: RoomClientID) {
        let mut interval = interval(HEARTBEAT_INTERVAL);
        let hb = Arc::clone(&self.hb);
        let mut session = self.session.clone();
        let room_id = self.room_id;

        actix_web::rt::spawn(async move {
            loop {
                interval.tick().await;

                if Instant::now().duration_since(*hb.lock().unwrap()) > CLIENT_TIMEOUT {
                    debug!(
                        "[id:{}, room_id:{}] Disconnecting failed heartbeat",
                        client_id, room_id
                    );
                    // TODO: Remove from Room
                    ws_manager.write().unwrap().ws_sessions.remove(&client_id);
                    let _ = session.close(None).await;
                    break;
                }

                if session.ping(b"PING").await.is_err() {
                    break;
                }
            }
        });
    }

    async fn send_in_room(
        &self,
        ws_manager: Arc<RwLock<SharifyWsManager>>,
        clients: Vec<RoomClientID>,
        msg: impl Into<String>,
    ) {
        let msg = msg.into();
        let guard = ws_manager.read().unwrap();
        let iter = guard
            .ws_sessions
            .iter()
            .filter_map(|(id, instance)| {
                if clients.contains(id) {
                    Some((id.clone(), instance.session.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        drop(guard);

        for (id, mut session) in iter {
            if session.text(msg.clone()).await.is_err() {
                ws_manager.write().unwrap().ws_sessions.remove(&id);
            }
        }
    }
}
