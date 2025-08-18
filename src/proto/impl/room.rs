use std::time::Instant;

use uuid::Uuid;

use crate::proto;
use crate::proto::cmd::command_response;
use crate::sharify::room;
use crate::sharify::spotify::Spotify;

impl From<room::LogType> for i32 {
    fn from(log: room::LogType) -> Self {
        match log {
            room::LogType::Other => 0,
            room::LogType::Kick => 1,
            room::LogType::Ban => 2,
        }
    }
}

impl From<i32> for room::LogType {
    fn from(log: i32) -> Self {
        match log {
            0 => Self::Other,
            1 => Self::Kick,
            2 => Self::Ban,
            _ => unreachable!(),
        }
    }
}

impl From<proto::room::Log> for room::Log {
    fn from(log: proto::room::Log) -> Self {
        Self {
            r#type: log.r#type.into(),
            details: log.details,
        }
    }
}

impl From<room::Log> for proto::room::Log {
    fn from(log: room::Log) -> Self {
        Self {
            r#type: log.r#type.into(),
            details: log.details,
        }
    }
}

impl From<proto::cmd::command_response::Type> for room::RoomError {
    fn from(err: proto::cmd::command_response::Type) -> Self {
        let command_response::Type::GenericError(error) = err else {
            unreachable!();
        };

        Self { error }
    }
}

impl From<room::RoomError> for proto::cmd::command_response::Type {
    fn from(err: room::RoomError) -> Self {
        Self::GenericError(err.error)
    }
}

impl From<proto::cmd::CommandResponse> for room::RoomError {
    fn from(err: proto::cmd::CommandResponse) -> Self {
        let Some(proto::cmd::command_response::Type::GenericError(error)) = err.r#type else {
            unreachable!();
        };

        Self { error }
    }
}

impl From<room::RoomError> for proto::cmd::CommandResponse {
    fn from(err: room::RoomError) -> Self {
        Self {
            r#type: Some(err.into()),
        }
    }
}

impl From<proto::room::RoomError> for room::RoomError {
    fn from(err: proto::room::RoomError) -> Self {
        Self { error: err.error }
    }
}

impl From<room::RoomError> for proto::room::RoomError {
    fn from(err: room::RoomError) -> Self {
        Self { error: err.error }
    }
}

impl From<proto::room::RoomTrack> for room::RoomTrack {
    fn from(track: proto::room::RoomTrack) -> Self {
        Self {
            user_id: track.user_id,
            track_id: track.track_id,
            track_name: track.track_name,
            track_duration: track.track_duration,
            last_checked: Instant::now(),
        }
    }
}

impl From<room::RoomTrack> for proto::room::RoomTrack {
    fn from(track: room::RoomTrack) -> Self {
        Self {
            user_id: track.user_id,
            track_id: track.track_id,
            track_name: track.track_name,
            track_duration: track.track_duration,
        }
    }
}

impl From<proto::room::RoomUser> for room::RoomUser {
    fn from(user: proto::room::RoomUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role_id: Uuid::from_slice(&user.role_id[..16]).unwrap(),
            is_connected: user.is_connected,
        }
    }
}

impl From<room::RoomUser> for proto::room::RoomUser {
    fn from(user: room::RoomUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role_id: user.role_id.into_bytes().into(),
            is_connected: user.is_connected,
        }
    }
}

impl From<proto::room::Room> for room::Room {
    fn from(room: proto::room::Room) -> Self {
        Self {
            id: Uuid::from_slice(&room.id[..16]).unwrap(),
            name: room.name,
            password: room.password,
            users: room.users.into_iter().map(Into::into).collect(),
            banned_users: room.banned_users,
            role_manager: room.role_manager.map(Into::into).unwrap_or_default(),
            tracks_queue: room.tracks_queue.into_iter().map(Into::into).collect(),
            logs: room.logs.into_iter().map(Into::into).collect(),
            max_users: room.max_users,
            inactive_for: None,
            spotify_handler: Spotify::default(),
        }
    }
}

impl From<room::Room> for proto::room::Room {
    fn from(room: room::Room) -> Self {
        Self {
            id: room.id.into_bytes().into(),
            name: room.name,
            password: room.password,
            users: room.users.into_iter().map(Into::into).collect(),
            banned_users: room.banned_users,
            role_manager: Some(room.role_manager.into()),
            tracks_queue: room.tracks_queue.into_iter().map(Into::into).collect(),
            logs: room.logs.into_iter().map(Into::into).collect(),
            max_users: room.max_users,
        }
    }
}
