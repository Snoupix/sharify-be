use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::proto::cmd::command;
use crate::proto::cmd::command_response;
use crate::proto::room::RoomTrack;
use crate::sharify::room::RoomManager;
use crate::sharify::room::{RoomClientID, RoomID};
use crate::sharify::spotify::Spotify;

#[async_trait]
trait Commands {
    type T;
    type Output;

    async fn search(self, name: String) -> Self::Output;
    async fn add_to_queue(self, track: RoomTrack) -> Self::Output;
    async fn set_volume(self, percentage: u8) -> Self::Output;
    async fn play_resume(self) -> Self::Output;
    async fn pause(self) -> Self::Output;
    async fn skip_next(self) -> Self::Output;
    async fn skip_previous(self) -> Self::Output;
    async fn seek_to_pos(self, pos: u64) -> Self::Output;
    async fn kick(self, opts: command::Kick) -> Self::Output;
    async fn ban(self, opts: command::Ban) -> Self::Output;
    async fn get_room(self) -> Self::Output;
}

pub struct Command {
    sharify_state: Arc<RwLock<RoomManager>>,
    client_id: RoomClientID,
    room_id: RoomID,
}

impl Command {
    pub fn new(
        sharify_state: Arc<RwLock<RoomManager>>,
        author_id: RoomClientID,
        room_id: RoomID,
    ) -> Self {
        Self {
            sharify_state,
            client_id: author_id,
            room_id,
        }
    }

    pub async fn process(
        self,
        cmd_type: command::Type,
    ) -> Result<Option<command_response::Type>, command_response::Type> {
        if !self.has_permission_to(&cmd_type).await {
            return Err(command_response::Type::Unauthorized(false));
        }

        match cmd_type {
            command::Type::Search(name) => self.search(name).await,
            command::Type::AddToQueue(room_track) => self.add_to_queue(room_track).await,
            command::Type::SetVolume(percentage) => self.set_volume(percentage as _).await,
            command::Type::PlayResume(_) => self.play_resume().await,
            command::Type::Pause(_) => self.pause().await,
            command::Type::SkipNext(_) => self.skip_next().await,
            command::Type::SkipPrevious(_) => self.skip_previous().await,
            command::Type::SeekToPos(pos) => self.seek_to_pos(pos).await,
            command::Type::Kick(opts) => self.kick(opts).await,
            command::Type::Ban(opts) => self.ban(opts).await,
            command::Type::GetRoom(_) => self.get_room().await,
        }
    }

    async fn has_permission_to(&self, cmd_type: &command::Type) -> bool {
        let guard = self.sharify_state.read().await;
        let Some(room) = guard.get_room(&self.room_id) else {
            return false;
        };
        let Some(client_role_id) = room.clients.iter().find_map(|client| {
            if client.id == self.client_id {
                Some(client.role_id)
            } else {
                None
            }
        }) else {
            return false;
        };
        let Some(role) = room.role_manager.get_role_by_id(&client_role_id) else {
            return false;
        };
        let perms = role.permissions;
        drop(guard);

        match *cmd_type {
            command::Type::Search(_) | command::Type::AddToQueue(_) => perms.can_add_song,
            command::Type::SetVolume(_)
            | command::Type::PlayResume(_)
            | command::Type::Pause(_)
            | command::Type::SkipNext(_)
            | command::Type::SkipPrevious(_)
            | command::Type::SeekToPos(_) => perms.can_use_controls,
            command::Type::Kick(_) | command::Type::Ban(_) => perms.can_manage_users,
            command::Type::GetRoom(_) => true,
        }
    }

    async fn get_spotify_handler(&self) -> Result<Spotify, command_response::Type> {
        let guard = self.sharify_state.read().await;

        let room = guard
            .get_room(&self.room_id)
            .ok_or(command_response::Type::GenericError(
                "Room not found".into(),
            ))?;

        Ok(room.spotify_handler.clone())
    }
}

#[async_trait]
impl Commands for Command {
    type T = command_response::Type;
    type Output = Result<Option<Self::T>, Self::T>;

    async fn search(self, name: String) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        let tracks = spotify
            .search_track(name)
            .await
            .map_err(Self::T::GenericError)?;

        Ok(Some(Self::T::SpotifyTracks(tracks.into())))
    }

    async fn add_to_queue(self, track: RoomTrack) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .add_track_to_queue(track.track_id)
            .await
            .map_err(Self::T::GenericError)?;

        Ok(None)
    }

    async fn set_volume(self, percentage: u8) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .set_volume(percentage)
            .await
            .map_err(Self::T::GenericError)?;

        Ok(None)
    }

    async fn play_resume(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify.play_resume().await.map_err(Self::T::GenericError)?;

        Ok(None)
    }

    async fn pause(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify.pause().await.map_err(Self::T::GenericError)?;

        Ok(None)
    }

    async fn skip_next(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify.skip_next().await.map_err(Self::T::GenericError)?;

        Ok(None)
    }

    async fn skip_previous(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .skip_previous()
            .await
            .map_err(Self::T::GenericError)?;

        Ok(None)
    }

    async fn seek_to_pos(self, pos: u64) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .seek_to_ms(pos)
            .await
            .map_err(Self::T::GenericError)?;

        Ok(None)
    }

    async fn kick(self, opts: command::Kick) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        guard
            .kick_client(self.room_id, &self.client_id, &opts.client_id, opts.reason)
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn ban(self, opts: command::Ban) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        guard
            .ban_client(self.room_id, &self.client_id, &opts.client_id, opts.reason)
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn get_room(self) -> Self::Output {
        let guard = self.sharify_state.read().await;

        let room = guard
            .get_room(&self.room_id)
            .ok_or(Self::T::GenericError("Room not found".into()))?
            .clone();

        Ok(Some(Self::T::Room(room.into())))
    }
}
