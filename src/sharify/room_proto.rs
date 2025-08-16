// Room to/from proto impl

use std::time::Instant;

use uuid::Uuid;

use super::room;
use crate::proto;
use crate::sharify::spotify::Spotify;

impl From<proto::room::RoomTrack> for room::RoomTrack {
    fn from(track: proto::room::RoomTrack) -> Self {
        Self {
            client_id: track.client_id,
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
            client_id: track.client_id,
            track_id: track.track_id,
            track_name: track.track_name,
            track_duration: track.track_duration,
        }
    }
}

impl From<proto::room::RoomClient> for room::RoomClient {
    fn from(client: proto::room::RoomClient) -> Self {
        Self {
            id: client.id,
            username: client.username,
            role_id: Uuid::from_slice_le(&client.role_id[..16]).unwrap(),
            is_connected: client.is_connected,
        }
    }
}

impl From<room::RoomClient> for proto::room::RoomClient {
    fn from(client: room::RoomClient) -> Self {
        Self {
            id: client.id,
            username: client.username,
            role_id: client.role_id.to_bytes_le().into(),
            is_connected: client.is_connected,
        }
    }
}

impl From<proto::room::Room> for room::Room {
    fn from(room: proto::room::Room) -> Self {
        Self {
            id: Uuid::from_slice_le(&room.id[..16]).unwrap(),
            name: room.name,
            password: room.password,
            clients: room.clients.into_iter().map(Into::into).collect(),
            banned_clients: room.banned_clients,
            role_manager: room.role_manager.map(Into::into).unwrap_or_default(),
            tracks_queue: room.tracks_queue.into_iter().map(Into::into).collect(),
            max_clients: room.max_clients,
            inactive_for: None,
            spotify_handler: Spotify::default(),
        }
    }
}

impl From<room::Room> for proto::room::Room {
    fn from(room: room::Room) -> Self {
        Self {
            id: room.id.to_bytes_le().into(),
            name: room.name,
            password: room.password,
            clients: room.clients.into_iter().map(Into::into).collect(),
            banned_clients: room.banned_clients,
            role_manager: Some(room.role_manager.into()),
            tracks_queue: room.tracks_queue.into_iter().map(Into::into).collect(),
            max_clients: room.max_clients,
        }
    }
}
