use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use actix::clock;
use actix_web::{web, Error as ActixError, HttpRequest, HttpResponse};
use actix_ws::{AggregatedMessage, Session};
use prost::Message as _;
use tokio::sync::{mpsc, Mutex, RwLock};

use crate::proto::cmd::{command, command_response, Command, CommandResponse};
use crate::sharify::room::{RoomID, RoomManager, RoomUserID};
use crate::sharify::utils;
use crate::sharify::websocket_cmds::Command as WSCmd;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const USER_WS_TIMEOUT: Duration = Duration::from_secs(HEARTBEAT_INTERVAL.as_secs() * 2);

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

// TODO future: Make a UserID map to a Vec<SharifyWsInstance> for 2 reasons:
// 1. The user can have multiple tabs open with the same session instead of overriding
// 2. The user could be on 2 different rooms (bigger feature)
#[derive(Default)]
pub struct SharifyWsManager {
    /// Maps a user_id to its Instance (ws_session, room_id, heartbeat)
    ws_sessions: HashMap<RoomUserID, SharifyWsInstance>,
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
        path: web::Path<(RoomID, RoomUserID)>,
    ) -> Result<HttpResponse, ActixError> {
        let (room_id, user_id) = path.into_inner();
        let state_guard = sharify_state.read().await;
        let room = state_guard
            .get_room(&room_id)
            .ok_or(actix_web::error::ErrorBadRequest(format!(
                "Room {} does not exist",
                room_id
            )))?;

        let user = room.users.iter().find(|e| e.id == user_id);

        if user.is_none() || room.banned_users.contains(&user_id) {
            return Err(actix_web::error::ErrorUnauthorized(String::from(
                "You are not allowed to join this room",
            )));
        }

        if let Some(Self { session, .. }) = sharify_ws_manager
            .write()
            .await
            .ws_sessions
            .remove(&user_id)
        {
            let _ = session.close(None).await;
        }

        drop(state_guard);

        let mut sharify_guard = sharify_state.write().await;
        if let Err(e) = sharify_guard.set_ws_user_state(room_id, &user_id, true) {
            return Err(actix_web::error::ErrorBadRequest(format!("WS Error: {e}")));
        }

        debug!(
            "WS Debug: Starting ws session for roomID {} and userID {}",
            room_id, user_id
        );

        let (res, mut session, stream) = actix_ws::handle(&req, body)?;
        let _self = Self::new(room_id, session.clone());

        // max 128kb stream
        let mut stream = stream.max_frame_size(1024 * 128).aggregate_continuations();

        sharify_ws_manager
            .write()
            .await
            .ws_sessions
            .insert(user_id.clone(), _self.clone());

        _self.init_heartbeat(Arc::clone(&sharify_ws_manager), user_id.clone());

        {
            let sharify_state = Arc::clone(&sharify_state);
            let ws_manager = Arc::clone(&sharify_ws_manager);

            actix_rt::spawn(async move {
                // TODO: Rework this comment
                // This means that it is the owner so we initiate the refresh token timeout and the data
                // fetching interval and the lobby data interval
                let mut data_fetching_guard = crate::DATA_FETCHING_INTERVALS
                    .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
                    .lock()
                    .await;

                if data_fetching_guard.contains_key(&room_id) {
                    return;
                }

                let (tx, mut rx) = mpsc::channel::<()>(1);

                data_fetching_guard.insert(room_id, tx);

                drop(data_fetching_guard);

                let mut interval = clock::interval(crate::SPOTIFY_FETCHING_INTERVAL);
                // let guard = sharify_state.read().await;
                // let room = guard.get_room(&room_id).unwrap();
                // let timeout: i64 = room.spotify_handler.tokens.expires_in.clone().into();
                // drop(guard);

                // TODO Impl refresh token loop

                loop {
                    tokio::select! {
                        _ = rx.recv() => {
                            break;
                        }
                        _ = interval.tick() => {
                                let mut guard = sharify_state.write().await;
                                let Some(room) = guard.get_room_mut(&room_id) else {
                                    break;
                                };

                                let (previous, state, next) = tokio::join!(
                                    room.spotify_handler.get_recent_tracks(Some(10)),
                                    room.spotify_handler.get_current_playback_state(),
                                    room.spotify_handler.get_next_tracks(),
                                );

                                if let Err(ref err) = previous {
                                    error!("Failed to fetch recent tracks for room {room_id}: {err}");
                                }

                                if let Err(ref err) = state {
                                    error!("Failed to fetch playback state for room {room_id}: {err}");
                                }

                                if let Err(ref err) = next {
                                    error!("Failed to fetch next tracks (queue) for room {room_id}: {err}");
                                }

                                if previous.is_err() || state.is_err() || next.is_err() {
                                    // TODO: Destroy Room ?
                                    break;
                                }

                                let ws_guard = ws_manager.read().await;
                                let room_users = ws_guard
                                    .ws_sessions
                                    .iter()
                                    .filter_map(|(id, instance)| {
                                        if instance.room_id == room_id {
                                            Some((id.clone(), instance.session.clone()))
                                        } else {
                                            None
                                        }
                                    }).collect::<Vec<_>>();

                                drop(ws_guard);

                                let cmd = CommandResponse {
                                    r#type: Some(
                                        command_response::Type::SpotifyPlaybackState(command_response::SpotifyPlaybackState {
                                            previous_tracks: Some(previous.unwrap().into()),
                                            state: state.unwrap().map(Into::into),
                                            next_tracks: Some(next.unwrap().into()),
                                        }
                                    ))
                                };

                                let mut buf = Vec::new();

                                cmd.encode(&mut buf).unwrap();

                                for (room_user_id, mut session) in room_users.into_iter() {
                                    Self::send_binary(
                                        &mut session,
                                        &room_user_id,
                                        Arc::clone(&ws_manager),
                                        buf.clone()
                                    ).await;
                                }
                        }
                    }
                }
            });
        }

        {
            let sharify_state = Arc::clone(&sharify_state);
            let ws_manager = Arc::clone(&sharify_ws_manager);
            let user_id = user_id.clone();

            actix_rt::spawn(async move {
                let mut buf = Vec::new();
                let mut interval = clock::interval(crate::DATA_FETCHING_INTERVAL);

                loop {
                    buf.clear();
                    interval.tick().await;

                    let guard = sharify_state.read().await;
                    let Some(room) = guard.get_room(&room_id) else {
                        break;
                    };

                    let cmd = CommandResponse {
                        r#type: Some(command_response::Type::Room(room.clone().into())),
                    };

                    // We can safely unwrap here since it cannot logically fail and if it does, it
                    // better break everything now.
                    cmd.encode(&mut buf).unwrap();

                    let mut ws_guard = ws_manager.write().await;
                    let Some(SharifyWsInstance { session, .. }) =
                        ws_guard.ws_sessions.get_mut(&user_id)
                    else {
                        break;
                    };

                    if !Self::send_binary(session, &user_id, Arc::clone(&ws_manager), buf.clone())
                        .await
                    {
                        break;
                    }
                }

                if let Some(SharifyWsInstance { session, .. }) =
                    ws_manager.write().await.ws_sessions.remove(&user_id)
                {
                    let _ = session.close(None).await;
                }
            });
        }

        {
            let hb = Arc::clone(&_self.hb);
            let sharify_state = Arc::clone(&sharify_state);
            let ws_manager = Arc::clone(&sharify_ws_manager);

            actix_rt::spawn(async move {
                while let Some(Ok(msg)) = stream.recv().await {
                    match msg {
                        AggregatedMessage::Ping(bytes) => {
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        AggregatedMessage::Text(string) => {
                            info!("Relaying text, {string}");
                            let guard = sharify_state.read().await;
                            let Some(room) = guard.get_room(&room_id) else {
                                continue;
                            };
                            let users = room.users.iter().map(|c| c.id.clone()).collect();

                            drop(guard);

                            Self::send_in_room(Arc::clone(&ws_manager), users, string).await;
                        }
                        AggregatedMessage::Close(_) => {
                            break;
                        }
                        AggregatedMessage::Pong(_) => {
                            *hb.lock().await = Instant::now();
                        }
                        AggregatedMessage::Binary(bytes) => {
                            let Ok(command) = Command::decode(bytes) else {
                                debug!(
                                    "Unrecognized command from user: {}",
                                    utils::decode_user_email(&user_id)
                                );
                                continue;
                            };
                            let Some(cmd_type) = command.r#type else {
                                continue;
                            };

                            let mut ws_guard = ws_manager.write().await;
                            let Some(SharifyWsInstance { session, .. }) =
                                ws_guard.ws_sessions.get_mut(&user_id)
                            else {
                                continue;
                            };

                            let ws_cmd =
                                WSCmd::new(Arc::clone(&sharify_state), user_id.clone(), room_id);

                            match ws_cmd.process(cmd_type.clone()).await {
                                // Ignore the Result until I might need to do smth differently based on it
                                Ok(Some(response)) | Err(response) => {
                                    let mut buf = Vec::new();
                                    response.encode(&mut buf);

                                    if !Self::send_binary(
                                        session,
                                        &user_id,
                                        Arc::clone(&ws_manager),
                                        buf,
                                    )
                                    .await
                                    {
                                        debug!("Failed to send command response to user {user_id}. WS session closed");
                                    }
                                }
                                Ok(None) => {
                                    let is_ban = matches!(cmd_type, command::Type::Ban(_));

                                    match cmd_type {
                                        command::Type::Kick(command::Kick { reason, .. })
                                        | command::Type::Ban(command::Ban { reason, .. }) => {
                                            if let Some(mut instance) =
                                                ws_guard.ws_sessions.remove(&user_id)
                                            {
                                                let mut buf = Vec::new();

                                                let cmd = if is_ban {
                                                    command_response::Type::Ban(
                                                        command_response::Ban { reason },
                                                    )
                                                } else {
                                                    command_response::Type::Kick(
                                                        command_response::Kick { reason },
                                                    )
                                                };

                                                cmd.encode(&mut buf);

                                                let _ = SharifyWsInstance::send_binary(
                                                    &mut instance.session,
                                                    &user_id,
                                                    Arc::clone(&ws_manager),
                                                    buf,
                                                )
                                                .await;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    };
                }

                // TODO: Remove from Room
                if let Some(SharifyWsInstance { session, .. }) =
                    ws_manager.write().await.ws_sessions.remove(&user_id)
                {
                    let _ = session.close(None).await;
                }
            });
        }

        Ok(res)
    }

    fn init_heartbeat(&self, ws_manager: Arc<RwLock<SharifyWsManager>>, user_id: RoomUserID) {
        let mut interval = clock::interval(HEARTBEAT_INTERVAL);
        let hb = Arc::clone(&self.hb);
        let mut session = self.session.clone();
        let room_id = self.room_id;

        actix_web::rt::spawn(async move {
            loop {
                interval.tick().await;

                if Instant::now().duration_since(*hb.lock().await) > USER_WS_TIMEOUT {
                    debug!(
                        "[id:{}, room_id:{}] Disconnecting failed heartbeat",
                        user_id, room_id
                    );
                    // TODO: Remove from Room
                    if let Some(SharifyWsInstance { session, .. }) =
                        ws_manager.write().await.ws_sessions.remove(&user_id)
                    {
                        let _ = session.close(None).await;
                    }
                    break;
                }

                if session.ping(b"PING").await.is_err() {
                    break;
                }
            }
        });
    }

    /// Returns false when session is closed and has been removed
    async fn send_text(
        session: &mut Session,
        user_id: &RoomUserID,
        ws_manager: Arc<RwLock<SharifyWsManager>>,
        msg: impl Into<String>,
    ) -> bool {
        if session.text(msg.into()).await.is_err() {
            ws_manager.write().await.ws_sessions.remove(user_id);
            return false;
        }

        true
    }

    /// Returns false when session is closed and has been removed
    async fn send_binary(
        session: &mut Session,
        user_id: &RoomUserID,
        ws_manager: Arc<RwLock<SharifyWsManager>>,
        buf: impl Into<web::Bytes>,
    ) -> bool {
        if session.binary(buf).await.is_err() {
            ws_manager.write().await.ws_sessions.remove(user_id);
            return false;
        }

        true
    }

    async fn send_in_room(
        ws_manager: Arc<RwLock<SharifyWsManager>>,
        users: Vec<RoomUserID>,
        msg: impl Into<String>,
    ) {
        let msg = msg.into();
        let guard = ws_manager.read().await;
        let iter = guard
            .ws_sessions
            .iter()
            .filter_map(|(id, instance)| {
                if users.contains(id) {
                    Some((id.clone(), instance.session.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        drop(guard);

        for (id, mut session) in iter {
            Self::send_text(&mut session, &id, Arc::clone(&ws_manager), msg.clone()).await;
        }
    }
}
