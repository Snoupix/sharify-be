use uuid::Uuid;

use crate::proto;
use crate::sharify::room;

impl From<room::LogType> for i32 {
    fn from(log: room::LogType) -> Self {
        match log {
            room::LogType::Other => 0,
            room::LogType::Kick => 1,
            room::LogType::Ban => 2,
            room::LogType::AddTrack => 3,
            room::LogType::JoinRoom => 4,
            room::LogType::LeaveRoom => 5,
            room::LogType::UsernameChange => 6,
        }
    }
}

impl From<i32> for room::LogType {
    fn from(log: i32) -> Self {
        match log {
            0 => Self::Other,
            1 => Self::Kick,
            2 => Self::Ban,
            3 => Self::AddTrack,
            4 => Self::JoinRoom,
            5 => Self::LeaveRoom,
            6 => Self::UsernameChange,
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

impl From<room::RoomError> for i32 {
    fn from(err: room::RoomError) -> Self {
        match err {
            room::RoomError::RoomCreationFail => 0,
            room::RoomError::RoomNotFound => 1,
            room::RoomError::RoomUserNotFound => 2,
            room::RoomError::RoleNotFound => 3,
            room::RoomError::Unauthorized => 4,
            room::RoomError::TrackNotFound => 5,
            room::RoomError::RoomFull => 6,
            room::RoomError::UserBanned => 7,
            room::RoomError::UserIDExists => 8,
            room::RoomError::Unreachable => 9,
        }
    }
}

impl From<i32> for room::RoomError {
    fn from(log: i32) -> Self {
        match log {
            0 => room::RoomError::RoomCreationFail,
            1 => room::RoomError::RoomNotFound,
            2 => room::RoomError::RoomUserNotFound,
            3 => room::RoomError::RoleNotFound,
            4 => room::RoomError::Unauthorized,
            5 => room::RoomError::TrackNotFound,
            6 => room::RoomError::RoomFull,
            7 => room::RoomError::UserBanned,
            8 => room::RoomError::UserIDExists,
            9 => room::RoomError::Unreachable,
            _ => unreachable!(),
        }
    }
}

impl From<room::RoomError> for proto::room::RoomError {
    fn from(err: room::RoomError) -> Self {
        match err {
            room::RoomError::RoomCreationFail => Self::RoomCreationFail,
            room::RoomError::RoomNotFound => Self::RoomNotFound,
            room::RoomError::RoomUserNotFound => Self::RoomUserNotFound,
            room::RoomError::RoleNotFound => Self::RoleNotFound,
            room::RoomError::Unauthorized => Self::Unauthorized,
            room::RoomError::TrackNotFound => Self::TrackNotFound,
            room::RoomError::RoomFull => Self::RoomFull,
            room::RoomError::UserBanned => Self::UserBanned,
            room::RoomError::UserIDExists => Self::UserIdExists,
            room::RoomError::Unreachable => Self::Unreachable,
        }
    }
}

impl From<proto::room::RoomError> for room::RoomError {
    fn from(err: proto::room::RoomError) -> Self {
        match err {
            proto::room::RoomError::RoomCreationFail => Self::RoomCreationFail,
            proto::room::RoomError::RoomNotFound => Self::RoomNotFound,
            proto::room::RoomError::RoomUserNotFound => Self::RoomUserNotFound,
            proto::room::RoomError::RoleNotFound => Self::RoleNotFound,
            proto::room::RoomError::Unauthorized => Self::Unauthorized,
            proto::room::RoomError::TrackNotFound => Self::TrackNotFound,
            proto::room::RoomError::RoomFull => Self::RoomFull,
            proto::room::RoomError::UserBanned => Self::UserBanned,
            proto::room::RoomError::UserIdExists => Self::UserIDExists,
            proto::room::RoomError::Unreachable => Self::Unreachable,
        }
    }
}

impl From<room::RoomError> for proto::cmd::command_response::Type {
    fn from(err: room::RoomError) -> Self {
        Self::RoomError(err.into())
    }
}

impl From<room::RoomError> for proto::cmd::CommandResponse {
    fn from(err: room::RoomError) -> Self {
        Self {
            r#type: Some(err.into()),
        }
    }
}

impl From<proto::room::RoomTrack> for room::RoomTrack {
    fn from(track: proto::room::RoomTrack) -> Self {
        Self {
            user_id: track.user_id,
            track_id: track.track_id,
            track_name: track.track_name,
            track_duration: track.track_duration,
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
        Self::from_proto_unsafe(room)
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
            max_users: room.max_users as _,
        }
    }
}
