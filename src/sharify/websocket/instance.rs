use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use actix_rt::time;
use actix_web::web::{self, Bytes};
use actix_web::{HttpRequest, HttpResponse, Responder};
use actix_ws::{AggregatedMessage, AggregatedMessageStream, CloseCode, CloseReason, Session};
use chrono::TimeDelta;
use prost::Message as _;
use tokio::sync::{Mutex, RwLock, mpsc};

use super::commands::{Command as WSCmd, StateImpact};
use crate::match_flags;
use crate::proto::cmd::{Command, CommandResponse, command, command_response};
use crate::sharify::room::{INACTIVE_ROOM_MINS, RoomError, RoomID, RoomUserID};
use crate::sharify::room_manager::RoomManager;
use crate::sharify::spotify::{self, SpotifyError};
use crate::sharify::utils::*;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// 2 times the HEARTBEAT_INTERVAL because we handle HB and Messages on the same loop and a message
///   has priority so if the HB is skipped once, it's safe but its unlikley be a problem
const USER_WS_TIMEOUT: Duration = Duration::from_secs(HEARTBEAT_INTERVAL.as_secs() * 2);

pub struct SharifyWsInstance {
    session: Session,
    room_id: RoomID,
    hb: Arc<Mutex<Instant>>,
    // This is true when the Client responded at the first ping
    // sent so the instance can recieve its initial data
    is_ready: bool,

    ws_mgr: Arc<RwLock<SharifyWsManager>>,
    state_mgr: Arc<RwLock<RoomManager>>,
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
/// Maps a user_id to its SharifyWsInstance
pub type SharifyWsManager = HashMap<RoomUserID, SharifyWsInstance>;

impl SharifyWsInstance {
    fn new(
        room_id: RoomID,
        session: Session,
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
    ) -> Self {
        SharifyWsInstance {
            hb: Arc::new(Mutex::new(Instant::now())),
            is_ready: false,
            room_id,
            session,
            ws_mgr,
            state_mgr,
        }
    }

    pub async fn init(
        req: HttpRequest,
        body: web::Payload,
        ws_mgr: web::Data<Arc<RwLock<SharifyWsManager>>>,
        state_mgr: web::Data<Arc<RwLock<RoomManager>>>,
        path: web::Path<(RoomID, RoomUserID)>,
    ) -> actix_web::Result<impl Responder> {
        let (room_id, user_id) = path.into_inner();
        let state_guard = state_mgr.read().await;
        let Some(room) = state_guard.get_room(&room_id) else {
            return Ok(HttpResponse::BadRequest().body(format!("Room {} does not exist", room_id)));
        };

        let are_room_threads_init = room.are_threads_initiated;

        let Some(user) = room.users.iter().find(|e| e.id == user_id) else {
            // User should have joined the room before WS init
            return Ok(HttpResponse::Unauthorized().finish());
        };

        let username = user.username.clone();

        if let Some(instance) = ws_mgr.write().await.remove(&user_id) {
            let _ = instance.session.close(None).await;
        }

        drop(state_guard);

        {
            let mut state_guard = state_mgr.write().await;

            if let Err(e) = state_guard.set_ws_user_state(room_id, &user_id, true) {
                return Ok(HttpResponse::InternalServerError().body(format!("{e:?}")));
            }

            if !are_room_threads_init {
                state_guard
                    .get_room_mut(&room_id)
                    .unwrap()
                    .are_threads_initiated = true;
            }
        }

        debug!(
            "[WS] Starting ws session for roomID {} and userID {}",
            room_id, user_id
        );

        let (res, session, stream) = actix_ws::handle(&req, body)?;
        let this = Self::new(
            room_id,
            session,
            Arc::clone(&ws_mgr),
            Arc::clone(&state_mgr),
        );

        // max 128kb stream
        let stream = stream.max_frame_size(1024 * 128).aggregate_continuations();

        // WS Instance scoped thread(s)
        this.init_main_loop(stream, user_id.clone());

        this.send_data_when_ready(user_id.clone());

        // Room scoped thread(s)
        if !are_room_threads_init {
            // Avoid fetching anything with Spotify on integration/unit tests
            if !cfg!(test) {
                // FIXME? ATM 5 is kinda arbitrary to avoid senders to be blocked but I may have to
                // think deeper about this buffer len
                let (tx, rx) = mpsc::channel(5);

                {
                    state_mgr
                        .write()
                        .await
                        .get_room_mut(&room_id)
                        .expect("Unreachable error: Room should exists")
                        .init_spotify_tick_tx(tx);
                }

                this.init_spotify_data_loop(rx);
            }

            this.init_room_activity_check_loop();

        // New Room user entered
        } else {
            let mut buf = Vec::new();

            let cmd = CommandResponse {
                r#type: Some(command_response::Type::NewUserJoined(username)),
            };

            cmd.encode(&mut buf).unwrap();

            Self::send_in_room(Arc::clone(&ws_mgr), room_id, buf).await;
        }

        ws_mgr.write().await.insert(user_id, this);

        Ok(res)
    }

    /// Handles MessageAggregator (so, Message stream) and Heartbeat
    /// intervals with a priority for message handling
    fn init_main_loop(&self, mut stream: AggregatedMessageStream, user_id: RoomUserID) {
        let ws_mgr = Arc::clone(&self.ws_mgr);
        let state_mgr = Arc::clone(&self.state_mgr);
        let mut interval = time::interval(HEARTBEAT_INTERVAL);
        let hb = Arc::clone(&self.hb);
        let mut session = self.session.clone();
        let room_id = self.room_id;

        actix_rt::spawn(async move {
            loop {
                tokio::select! {
                    biased;

                    stream_msg = stream.recv() => {
                        match stream_msg {
                            Some(Ok(msg)) => {
                                match msg {
                                    AggregatedMessage::Ping(bytes) => {
                                        if session.pong(&bytes).await.is_err() {
                                            break;
                                        }
                                    }
                                    AggregatedMessage::Pong(_) => {
                                        if let Some(instance) = ws_mgr.write().await.get_mut(&user_id) {
                                            instance.is_ready = true;
                                        }

                                        *hb.lock().await = Instant::now();
                                    }
                                    AggregatedMessage::Text(_) => {}
                                    AggregatedMessage::Close(_) => {
                                        break;
                                    }
                                    AggregatedMessage::Binary(bytes) => {
                                        if !Self::handle_binary_message(
                                            bytes,
                                            Arc::clone(&ws_mgr),
                                            Arc::clone(&state_mgr),
                                            room_id,
                                            &user_id
                                        ).await {
                                            break;
                                        }
                                    }
                                }
                            }
                            // Ignore protocol error for the moment
                            None | Some(Err(_)) => break
                        }
                    }
                    _ = interval.tick() => {
                        if Instant::now().duration_since(*hb.lock().await) > USER_WS_TIMEOUT {
                            debug!(
                                "[WS] Disconnecting failed heartbeat email:{}, id:{}, room_id:{}",
                                decode_user_email(&user_id),
                                user_id,
                                room_id
                            );
                            break;
                        }

                        if session.ping(b"PING").await.is_err() {
                            break;
                        }
                    }
                }
            }

            Self::close_session(Arc::clone(&ws_mgr), Arc::clone(&state_mgr), user_id, None).await;
        });
    }

    /// Returns wether the aggregator loop should or shouldn't continue
    async fn handle_binary_message(
        bytes: Bytes,
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        room_id: RoomID,
        user_id: &RoomUserID,
    ) -> bool {
        let Ok(command) = Command::decode(bytes) else {
            debug!(
                "Unrecognized command from user: {}",
                decode_user_email(user_id)
            );
            return true;
        };
        let Some(cmd_type) = command.r#type else {
            return true;
        };

        let ws_guard = ws_mgr.read().await;
        let Some(mut session) = ws_guard
            .get(user_id)
            .map(|instance| instance.session.clone())
        else {
            return false;
        };

        drop(ws_guard);

        let should_room_be_closed = state_mgr
            .read()
            .await
            .is_user_an_owner_and_alone(room_id, user_id);

        let ws_cmd = WSCmd::new(
            Arc::clone(&state_mgr),
            user_id.clone(),
            room_id,
            cmd_type.clone(),
        );

        let processed_cmd = ws_cmd.process().await;

        // Handle state impact first
        if let (Ok(_), state_impact) = &processed_cmd {
            match state_impact {
                StateImpact::Nothing => {}
                impact @ StateImpact::Room | impact @ StateImpact::Both(_) => {
                    if let StateImpact::Both(spotify_fetching) = impact {
                        let spotify_fetching = *spotify_fetching;
                        let ws_mgr = Arc::clone(&ws_mgr);
                        let state_mgr = Arc::clone(&state_mgr);

                        // This is a bit ugly but wesocket is so fast that
                        // Spotify current playback data is not synced yet
                        //
                        // The room data needs to happen after since the command
                        // could have been Skip(Next|Previous) and the TracksQueue
                        // has to be sync
                        actix_rt::spawn(async move {
                            actix_rt::time::sleep(Duration::from_millis(500)).await;

                            let _ = Self::send_spotify_state_in_room(
                                Arc::clone(&ws_mgr),
                                Arc::clone(&state_mgr),
                                room_id,
                                spotify_fetching,
                            )
                            .await;

                            Self::send_room_data_in_room(
                                Arc::clone(&ws_mgr),
                                Arc::clone(&state_mgr),
                                room_id,
                            )
                            .await;
                        });
                    }
                }
            }
        }

        // Then handle cmd result
        match processed_cmd {
            // Ignore the Result until I might need to do smth differently based on it
            (Ok(Some(response)), _) | (Err(response), _) => {
                let mut buf = Vec::new();
                response.encode(&mut buf);

                if !Self::send_binary(&mut session, user_id, Arc::clone(&ws_mgr), buf).await {
                    debug!("Failed to send command response to user {user_id}. WS session closed");
                }
            }
            (Ok(None), _) => {
                let is_ban = matches!(cmd_type, command::Type::Ban(_));

                match cmd_type {
                    command::Type::Kick(command::Kick { reason, user_id })
                    | command::Type::Ban(command::Ban { reason, user_id }) => {
                        if let Some(mut instance) = ws_mgr.write().await.remove(&user_id) {
                            let mut buf = Vec::new();

                            let cmd = if is_ban {
                                command_response::Type::Ban(command_response::Ban { reason })
                            } else {
                                command_response::Type::Kick(command_response::Kick { reason })
                            };

                            cmd.encode(&mut buf);

                            let _ = SharifyWsInstance::send_binary(
                                &mut instance.session,
                                &user_id,
                                Arc::clone(&ws_mgr),
                                buf,
                            )
                            .await;
                        }
                    }
                    command::Type::LeaveRoom(_) => {
                        Self::close_session(
                            Arc::clone(&ws_mgr),
                            Arc::clone(&state_mgr),
                            user_id.clone(),
                            None,
                        )
                        .await;

                        if should_room_be_closed.is_ok_and(|b| b) {
                            Self::close_room(
                                ws_mgr,
                                state_mgr,
                                room_id,
                                Some("No owner left to manage the room, closing...".into()),
                            )
                            .await;

                            return false;
                        }
                    }
                    _ => {}
                }
            }
        }

        true
    }

    fn send_data_when_ready(&self, user_id: RoomUserID) {
        let ws_mgr = Arc::clone(&self.ws_mgr);
        let state_mgr = Arc::clone(&self.state_mgr);

        actix_rt::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(500));

            loop {
                interval.tick().await;

                let (mut session, room_id) = {
                    let ws_guard = ws_mgr.read().await;
                    let Some(instance) = ws_guard.get(&user_id) else {
                        // Reachable if the client is dropped instantly
                        break;
                    };

                    if !instance.is_ready {
                        continue;
                    }

                    (instance.session.clone(), instance.room_id)
                };

                Self::send_room_data_in_room(Arc::clone(&ws_mgr), Arc::clone(&state_mgr), room_id)
                    .await;

                let mut buf = Vec::new();

                if let Err(err) = Self::send_spotify_state_in_room(
                    Arc::clone(&ws_mgr),
                    Arc::clone(&state_mgr),
                    room_id,
                    SPOTIFY_FETCH_TRACKS_Q | SPOTIFY_FETCH_PLAYBACK,
                )
                .await
                {
                    let cmd = CommandResponse {
                        r#type: Some(err.into()),
                    };

                    cmd.encode(&mut buf).unwrap();

                    Self::send_binary(&mut session, &user_id, Arc::clone(&ws_mgr), buf).await;
                }

                break;
            }
        });
    }

    fn init_room_activity_check_loop(&self) {
        let room_id = self.room_id;
        let state_mgr = Arc::clone(&self.state_mgr);

        actix_rt::spawn(async move {
            let mut interval = time::interval(crate::DATA_FETCHING_INTERVAL);

            loop {
                interval.tick().await;

                let mut guard = state_mgr.write().await;
                let Some(room) = guard.get_room_mut(&room_id) else {
                    break;
                };

                // No user connected to the Room
                if room.users.iter().filter(|u| u.is_connected).count() == 0 {
                    if room.inactive_for.is_some_and(|inactive| {
                        inactive.elapsed().as_secs() >= INACTIVE_ROOM_MINS as _
                    }) {
                        let _ = guard.delete_room(room_id, None);

                        break;
                    } else {
                        room.inactive_for = Some(Instant::now());
                    }
                } else {
                    room.inactive_for = None;
                }
            }

            let mut data_fetching_guard = crate::DATA_FETCHING_INTERVALS
                .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
                .lock()
                .await;

            // Break spotify_data_loop if it still exists
            if let Some(tx) = data_fetching_guard.remove(&room_id) {
                let _ = tx.send(()).await;
            }
        });
    }

    fn init_spotify_data_loop(&self, mut tick_rx: mpsc::Receiver<Duration>) {
        // Implicit copy to avoid self refs
        let room_id = self.room_id;
        let ws_mgr = Arc::clone(&self.ws_mgr);
        let state_mgr = Arc::clone(&self.state_mgr);

        actix_rt::spawn(async move {
            let mut data_fetching_guard = crate::DATA_FETCHING_INTERVALS
                .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
                .lock()
                .await;

            if data_fetching_guard.contains_key(&room_id) {
                error!(
                    "Unexpected error: Trying to start a spotify data loop while it already exists on that room id"
                );
                return;
            }

            let (tx, mut rx) = mpsc::channel::<()>(1);
            data_fetching_guard.insert(room_id, tx);

            drop(data_fetching_guard);

            if Self::send_spotify_state_in_room(
                Arc::clone(&ws_mgr),
                Arc::clone(&state_mgr),
                room_id,
                SPOTIFY_FETCH_PLAYBACK | SPOTIFY_FETCH_TRACKS_Q,
            )
            .await
            .is_err()
            {
                // FIXME? UX related
                // Most probably revoked tokens. They may have been refreshed from here or
                // elsewhere but the client holds stale/outdated tokens
                Self::close_room(
                    ws_mgr,
                    state_mgr,
                    room_id,
                    Some("Spotify request error. Closing room...".into()),
                )
                .await;

                return;
            }

            let sleep_fut =
                time::sleep_until(time::Instant::now() + spotify::DEFAULT_DATA_INTERVAL);

            tokio::pin!(sleep_fut);

            loop {
                tokio::select! {
                    biased;

                    _ = rx.recv() => {
                        break;
                    }
                    myb_tick = tick_rx.recv() => {
                        match myb_tick {
                            Some(tick) => {
                                debug!("Spotify sleeper reset to {}s", tick.as_secs());
                                sleep_fut.as_mut().reset(time::Instant::now() + tick);
                                continue;
                            }
                            None => break,
                        }
                    }
                    _ = &mut sleep_fut => {
                        if Self::send_spotify_state_in_room(
                            Arc::clone(&ws_mgr),
                            Arc::clone(&state_mgr),
                            room_id,
                            SPOTIFY_FETCH_PLAYBACK | SPOTIFY_FETCH_TRACKS_Q,
                        ).await.is_err() {
                            Self::close_room(
                                ws_mgr,
                                state_mgr,
                                room_id,
                                Some("Spotify request error. Closing room...".into()),
                            ).await;

                            break;
                        }
                    }
                }
            }
        });
    }

    /// Also handles refresh token fetch when expired
    ///
    /// Can fail if:
    ///     - Room not found
    ///     - Spotify endpoint fetch is err
    ///     - Refresh token fetch fail
    async fn send_spotify_state_in_room(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        room_id: RoomID,
        spotify_fetch_flags: SpotifyFetchT,
    ) -> Result<(), SpotifyError> {
        let mut guard = state_mgr.write().await;
        let Some(room) = guard.get_room_mut(&room_id) else {
            return Err(SpotifyError::Generic("Room not found".into()));
        };

        let now = chrono::Utc::now();
        let created_at = room
            .spotify_handler
            .tokens
            .created_at
            .to_datetime()
            .unwrap();
        let expires_at = created_at
            .checked_add_signed(TimeDelta::seconds(
                room.spotify_handler.tokens.expires_in as _,
            ))
            .unwrap();

        if now > expires_at
            && let Err(err) = room.spotify_handler.fetch_refresh_token().await
        {
            let mut buf = Vec::new();

            CommandResponse::from(err).encode(&mut buf).unwrap();

            Self::send_in_room(ws_mgr, room_id, buf).await;

            return Err(SpotifyError::Generic("Failed to refresh tokens".into()));
        }

        drop(guard);

        let cmd = match_flags!(
            spotify_fetch_flags,
            [SPOTIFY_FETCH_ALL; Self::fetch_spotify_all(Arc::clone(&ws_mgr), Arc::clone(&state_mgr), room_id)],
            [SPOTIFY_FETCH_PLAYBACK; Self::fetch_spotify_playback(Arc::clone(&ws_mgr), Arc::clone(&state_mgr), room_id)],
            [SPOTIFY_FETCH_TRACKS_Q; Self::fetch_spotify_tracks(Arc::clone(&ws_mgr), Arc::clone(&state_mgr), room_id)];
            [flags; panic!("Unhandled Spotify Fetch flags: {flags}")]
        );

        let mut buf = Vec::new();

        cmd.encode(&mut buf).unwrap();

        Self::send_in_room(Arc::clone(&ws_mgr), room_id, buf).await;

        Ok(())
    }

    async fn fetch_spotify_all(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        room_id: RoomID,
    ) -> Result<CommandResponse, SpotifyError> {
        let mut rate_limit = None;
        let mut guard = state_mgr.write().await;
        let Some(room) = guard.get_room_mut(&room_id) else {
            return Err(SpotifyError::Generic("Room not found".into()));
        };

        let (state, next, previous) = tokio::join!(
            room.spotify_handler.get_current_playback_state(),
            room.spotify_handler.get_next_tracks(),
            room.spotify_handler.get_recent_tracks(Some(10)),
        );

        if let Err(ref err) = previous {
            error!(
                "Failed to fetch recent tracks for room {room_id}: {}",
                String::from(err.clone())
            );

            if let SpotifyError::RateLimited(time) = err {
                rate_limit = Some(time);
            }
        }

        if let Err(ref err) = state {
            error!(
                "Failed to fetch playback state for room {room_id}: {}",
                String::from(err.clone())
            );

            if let SpotifyError::RateLimited(time) = err {
                rate_limit = Some(time);
            }
        }

        if let Err(ref err) = next {
            error!(
                "Failed to fetch next tracks (queue) for room {room_id}: {}",
                String::from(err.clone())
            );

            if let SpotifyError::RateLimited(time) = err {
                rate_limit = Some(time);
            }
        }

        if let Some(time) = rate_limit {
            let cmd = CommandResponse {
                r#type: Some(command_response::Type::SpotifyRateLimited(*time)),
            };

            let mut buf = Vec::new();

            cmd.encode(&mut buf).unwrap();

            Self::send_in_room(Arc::clone(&ws_mgr), room_id, buf).await;
        }

        if let Ok(Some(ref playback)) = state {
            if playback.is_playing
                && let Some(progress_ms) = playback.progress_ms
            {
                let mut rest_ms = playback.duration_ms - progress_ms;

                // If there's more than 2min left, add a fetch in the middle to keep sync with an
                // external spotify client/player
                if rest_ms > 1000 * 60 * 2 {
                    rest_ms /= 2;
                }

                room.set_spotify_tick(Duration::from_millis(rest_ms + spotify::FETCH_OFFSET_MS))
                    .await;
            } else {
                // Playtrack is not playing
                room.set_spotify_tick(spotify::DEFAULT_DATA_INTERVAL).await;
            }

            let _ = guard.remove_track_from_queue(room_id, playback.track_id.clone());
        }

        Ok(CommandResponse {
            r#type: Some(command_response::Type::SpotifyAllState(
                command_response::SpotifyAllState {
                    previous_tracks: previous.map(|v| Some(v.into())).unwrap_or_default(),
                    state: state.map(|v| v.map(Into::into)).unwrap_or_default(),
                    next_tracks: next.map(|v| Some(v.into())).unwrap_or_default(),
                },
            )),
        })
    }

    async fn fetch_spotify_tracks(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        room_id: RoomID,
    ) -> Result<CommandResponse, SpotifyError> {
        let mut rate_limit = None;
        let mut guard = state_mgr.write().await;
        let Some(room) = guard.get_room_mut(&room_id) else {
            return Err(SpotifyError::Generic("Room not found".into()));
        };

        let (next, previous) = tokio::join!(
            room.spotify_handler.get_next_tracks(),
            room.spotify_handler.get_recent_tracks(Some(10)),
        );

        if let Err(ref err) = previous {
            error!(
                "Failed to fetch recent tracks for room {room_id}: {}",
                String::from(err.clone())
            );

            if let SpotifyError::RateLimited(time) = err {
                rate_limit = Some(time);
            }
        }

        if let Err(ref err) = next {
            error!(
                "Failed to fetch next tracks (queue) for room {room_id}: {}",
                String::from(err.clone())
            );

            if let SpotifyError::RateLimited(time) = err {
                rate_limit = Some(time);
            }
        }

        if let Some(time) = rate_limit {
            let cmd = CommandResponse {
                r#type: Some(command_response::Type::SpotifyRateLimited(*time)),
            };

            let mut buf = Vec::new();

            cmd.encode(&mut buf).unwrap();

            Self::send_in_room(Arc::clone(&ws_mgr), room_id, buf).await;
        }

        Ok(CommandResponse {
            r#type: Some(command_response::Type::SpotifyTracksState(
                command_response::SpotifyTracksState {
                    previous_tracks: previous.map(|v| Some(v.into())).unwrap_or_default(),
                    next_tracks: next.map(|v| Some(v.into())).unwrap_or_default(),
                },
            )),
        })
    }

    async fn fetch_spotify_playback(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        room_id: RoomID,
    ) -> Result<CommandResponse, SpotifyError> {
        let mut rate_limit = None;
        let mut guard = state_mgr.write().await;
        let Some(room) = guard.get_room_mut(&room_id) else {
            return Err(SpotifyError::Generic("Room not found".into()));
        };

        let state = room.spotify_handler.get_current_playback_state().await;

        if let Err(ref err) = state {
            error!(
                "Failed to fetch playback state for room {room_id}: {}",
                String::from(err.clone())
            );

            if let SpotifyError::RateLimited(time) = err {
                rate_limit = Some(time);
            }
        }

        if let Some(time) = rate_limit {
            let cmd = CommandResponse {
                r#type: Some(command_response::Type::SpotifyRateLimited(*time)),
            };

            let mut buf = Vec::new();

            cmd.encode(&mut buf).unwrap();

            Self::send_in_room(Arc::clone(&ws_mgr), room_id, buf).await;
        }

        if let Ok(Some(ref playback)) = state {
            if playback.is_playing
                && let Some(progress_ms) = playback.progress_ms
            {
                let mut rest_ms = playback.duration_ms - progress_ms;

                // If there's more than 2min left, add a fetch in the middle to keep sync with an
                // external spotify client/player
                if rest_ms > 1000 * 60 * 2 {
                    rest_ms /= 2;
                }

                room.set_spotify_tick(Duration::from_millis(rest_ms + spotify::FETCH_OFFSET_MS))
                    .await;
            }

            let _ = guard.remove_track_from_queue(room_id, playback.track_id.clone());
        }

        Ok(CommandResponse {
            r#type: Some(command_response::Type::SpotifyPlaybackState(
                command_response::SpotifyPlaybackState {
                    state: state.map(|v| v.map(Into::into)).unwrap_or_default(),
                },
            )),
        })
    }

    async fn send_room_data_in_room(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        room_id: RoomID,
    ) {
        let mut buf = Vec::new();

        let cmd = CommandResponse {
            r#type: Some(match state_mgr.write().await.get_room_mut(&room_id) {
                None => command_response::Type::RoomError(
                    // TODO Unreachable ?
                    RoomError::RoomNotFound.into(),
                ),
                Some(room) => command_response::Type::Room(room.clone().into()),
            }),
        };

        cmd.encode(&mut buf).unwrap();

        Self::send_in_room(Arc::clone(&ws_mgr), room_id, buf).await;
    }

    /// Returns false when session is closed and has been removed
    async fn send_binary(
        session: &mut Session,
        user_id: &RoomUserID,
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        buf: impl Into<web::Bytes>,
    ) -> bool {
        if session.binary(buf).await.is_err() {
            ws_mgr.write().await.remove(user_id);
            return false;
        }

        true
    }

    async fn send_in_room(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        room_id: RoomID,
        buf: impl Into<web::Bytes> + Clone,
    ) {
        let ws_guard = ws_mgr.read().await;

        let room_users = ws_guard
            .iter()
            .filter_map(|(id, instance)| {
                if instance.room_id == room_id {
                    Some((id.clone(), instance.session.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        drop(ws_guard);

        for (room_user_id, mut session) in room_users {
            Self::send_binary(
                &mut session,
                &room_user_id,
                Arc::clone(&ws_mgr),
                buf.clone().into(),
            )
            .await;
        }
    }

    async fn close_session(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        user_id: RoomUserID,
        reason: Option<CloseReason>,
    ) {
        debug!(
            "[WS] Closing session email:{}, id:{}",
            decode_user_email(&user_id),
            user_id,
        );

        let Some(SharifyWsInstance {
            ref session,
            room_id,
            ..
        }) = ws_mgr.write().await.remove(&user_id)
        else {
            return;
        };

        let _ = session.clone().close(reason).await;

        let _ = state_mgr
            .write()
            .await
            .set_ws_user_state(room_id, &user_id, false);
    }

    async fn close_room(
        ws_mgr: Arc<RwLock<SharifyWsManager>>,
        state_mgr: Arc<RwLock<RoomManager>>,
        room_id: RoomID,
        reason: Option<String>,
    ) {
        let mut ws_guard = ws_mgr.write().await;

        let room_users_id = ws_guard
            .iter()
            .filter_map(|(id, instance)| {
                if instance.room_id == room_id {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for room_user_id in room_users_id {
            if let Some(instance) = ws_guard.remove(&room_user_id) {
                let _ = instance
                    .session
                    .close(Some(CloseReason {
                        code: CloseCode::Normal,
                        description: reason.clone(),
                    }))
                    .await;
            }
        }

        let _ = state_mgr.write().await.delete_room(room_id, None);
    }
}
