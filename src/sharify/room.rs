use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::proto;

use super::role::RoleManager;
use super::room_metadata::*;
use super::spotify::{SpotifyTokens, Timestamp};

pub(super) const MAX_USERS: usize = 15;
pub(super) const MAX_LOGS_LEN: usize = 25;
pub(super) const MAX_TRACKS_QUEUE_LEN: usize = 50;
pub(super) const INACTIVE_ROOM_MINS: u32 = 5;

// email / uuid allowed chars
pub(super) const MIN_EMAIL_CHAR: char = '-';
pub(super) const MAX_EMAIL_CHAR: char = 'z';

pub type RoomID = Uuid;
pub type RoomUserID = String;

#[derive(Clone, Debug, Serialize)]
pub struct Room {
    pub id: RoomID,
    pub name: String,
    pub password: String,
    pub users: Vec<RoomUser>,
    pub banned_users: Vec<RoomUserID>,
    /// Role hierarchy is: Most powerful role first, then less powerfull, then less...
    pub role_manager: RoleManager,
    // pub current_device: Option<SpotifyApi.UserDevice>,
    pub tracks_queue: VecDeque<RoomTrack>,
    pub max_users: usize,
    // TODO: Add log on every action
    /// Last 25 logs: Ban, Kick, Song added... (25 for memory purposes)
    pub logs: VecDeque<Log>,

    #[serde(skip)]
    pub(super) metadata: RoomMetadata,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Log {
    pub r#type: LogType,
    pub details: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LogType {
    Other,
    Kick,
    Ban,
    AddTrack,
    JoinRoom,
    LeaveRoom,
    UsernameChange,
}

impl Log {
    pub fn new(r#type: LogType, details: String) -> Self {
        Self { r#type, details }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialsInput {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u32,
    pub created_at: Timestamp,
}

impl From<CredentialsInput> for SpotifyTokens {
    fn from(val: CredentialsInput) -> Self {
        SpotifyTokens {
            access_token: val.access_token,
            refresh_token: val.refresh_token,
            expires_in: val.expires_in,
            created_at: val.created_at,
        }
    }
}

// TODO: On current track playing fetch => if the song matches the first [0] of the list, shift it
#[derive(Clone, Debug, Serialize)]
pub struct RoomTrack {
    pub user_id: RoomUserID,
    pub track_id: String,
    pub track_name: String,
    pub track_duration: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct RoomUser {
    pub id: RoomUserID,
    pub username: String,
    pub role_id: Uuid,
    pub is_connected: bool, // TODO: Handle this everywhere
}

impl PartialEq for RoomUser {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum RoomError {
    RoomCreationFail,
    RoomNotFound,
    RoomUserNotFound,
    RoleNotFound,
    Unauthorized,
    TrackNotFound,
    RoomFull,
    UserBanned,
    UserIDExists,
    Unreachable,
}

impl Room {
    /// This is a helper fn to create a Room struct from a proto Room
    /// but unsafe to use because of partially uninitialized fields.
    /// This is for testing purposes only
    pub fn from_proto_unsafe(room: proto::room::Room) -> Self {
        Self {
            id: Uuid::from_slice(&room.id[..16]).unwrap_or_default(),
            name: room.name,
            password: room.password,
            users: room.users.into_iter().map(Into::into).collect(),
            banned_users: room.banned_users,
            role_manager: room.role_manager.map(Into::into).unwrap_or_default(),
            tracks_queue: room.tracks_queue.into_iter().map(Into::into).collect(),
            logs: room.logs.into_iter().map(Into::into).collect(),
            max_users: room.max_users as _,
            metadata: RoomMetadata::new(SpotifyTokens::default()),
        }
    }

    pub fn to_json(&self) -> Value {
        json!(self)
    }
}

impl Deref for Room {
    type Target = RoomMetadata;

    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}

impl DerefMut for Room {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.metadata
    }
}
