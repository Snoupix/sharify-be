use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::proto::cmd::command;
use crate::proto::cmd::command_response;
use crate::sharify::room::{RoomError, RoomID, RoomUserID};
use crate::sharify::room_manager::RoomManager;
use crate::sharify::spotify::Spotify;
use crate::sharify::utils::*;

pub enum StateImpact {
    Nothing,
    Room,
    // Anything player related is gonna affect the room state (logs for example)
    Both(SpotifyFetchT),
}

#[async_trait]
trait Commands {
    type T;
    type Output;

    async fn get_room(self) -> Self::Output;
    async fn search(self, name: String) -> Self::Output;
    async fn add_to_queue(self, track: command::AddTrackToQueue) -> Self::Output;
    async fn set_volume(self, percentage: u8) -> Self::Output;
    async fn play_resume(self) -> Self::Output;
    async fn pause(self) -> Self::Output;
    async fn skip_next(self) -> Self::Output;
    async fn skip_previous(self) -> Self::Output;
    async fn seek_to_pos(self, pos: u64) -> Self::Output;
    async fn kick(self, opts: command::Kick) -> Self::Output;
    async fn ban(self, opts: command::Ban) -> Self::Output;
    async fn leave_room(self) -> Self::Output;
    async fn create_role(self, opts: command::CreateRole) -> Self::Output;
    async fn rename_role(self, opts: command::RenameRole) -> Self::Output;
    async fn delete_role(self, id: Vec<u8>) -> Self::Output;
}

pub struct Command {
    sharify_state: Arc<RwLock<RoomManager>>,
    user_id: RoomUserID,
    room_id: RoomID,
}

impl Command {
    pub fn new(
        sharify_state: Arc<RwLock<RoomManager>>,
        author_id: RoomUserID,
        room_id: RoomID,
    ) -> Self {
        Self {
            sharify_state,
            user_id: author_id,
            room_id,
        }
    }

    /// Returns a Protobuf response
    ///
    /// The Ok and Err variants contain the command response that is the result and the fail reason
    /// respectively. The second part is the StateImpact
    ///
    /// The Ok variant can have a None command response if the said command doesn't return data
    ///
    /// **StateImpact** tells what did the command impacted regarding the room state / player state
    /// to potentially inform room members
    ///
    /// For DX (pattern matching) purposes, the StateImpact is also on the Err variant even if it
    /// has no real sense because the command shouldn't have affected any state
    pub async fn process(
        self,
        cmd_type: command::Type,
    ) -> (
        Result<Option<command_response::Type>, command_response::Type>,
        StateImpact,
    ) {
        if !self.has_permission_to(&cmd_type).await {
            return (
                Err(command_response::Type::RoomError(
                    RoomError::Unauthorized.into(),
                )),
                StateImpact::Nothing,
            );
        }

        let cmd_impact = match &cmd_type {
            command::Type::GetRoom(_) | command::Type::Search(_) => StateImpact::Nothing,
            command::Type::DeleteRole(_)
            | command::Type::CreateRole(_)
            | command::Type::RenameRole(_)
            | command::Type::LeaveRoom(_)
            | command::Type::Kick(_)
            | command::Type::Ban(_) => StateImpact::Room,
            command::Type::AddToQueue(_)
            | command::Type::SetVolume(_)
            | command::Type::PlayResume(_)
            | command::Type::Pause(_)
            | command::Type::SkipNext(_)
            | command::Type::SkipPrevious(_)
            | command::Type::SeekToPos(_) => StateImpact::Both(match &cmd_type {
                command::Type::AddToQueue(_) => SPOTIFY_FETCH_TRACKS_Q,
                command::Type::SetVolume(_)
                | command::Type::PlayResume(_)
                | command::Type::Pause(_)
                | command::Type::SeekToPos(_) => SPOTIFY_FETCH_PLAYBACK,
                command::Type::SkipNext(_) | command::Type::SkipPrevious(_) => {
                    SPOTIFY_FETCH_TRACKS_Q | SPOTIFY_FETCH_PLAYBACK
                }
                _ => unreachable!(),
            }),
        };

        (
            match cmd_type {
                command::Type::GetRoom(_) => self.get_room().await,
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
                command::Type::LeaveRoom(_) => self.leave_room().await,
                command::Type::CreateRole(opts) => self.create_role(opts).await,
                command::Type::RenameRole(opts) => self.rename_role(opts).await,
                command::Type::DeleteRole(id) => self.delete_role(id).await,
            },
            cmd_impact,
        )
    }

    async fn has_permission_to(&self, cmd_type: &command::Type) -> bool {
        let guard = self.sharify_state.read().await;
        let Some(room) = guard.get_room(&self.room_id) else {
            return false;
        };
        let Some(user_role_id) = room.users.iter().find_map(|user| {
            if user.id == self.user_id {
                Some(user.role_id)
            } else {
                None
            }
        }) else {
            return false;
        };
        let Some(role) = room.role_manager.get_role_by_id(&user_role_id) else {
            return false;
        };

        let perms = role.permissions;

        if let command::Type::RenameRole(opts) = cmd_type {
            let Ok(role_id) = Uuid::from_slice(&opts.role_id[..16]) else {
                return false;
            };
            let Some(target_role) = room.role_manager.get_role_by_id(&role_id) else {
                return false;
            };

            if target_role >= role {
                return false;
            }
        }

        drop(guard);

        match *cmd_type {
            command::Type::GetRoom(_) | command::Type::LeaveRoom(_) => true,
            command::Type::Search(_) | command::Type::AddToQueue(_) => perms.can_add_song,
            command::Type::SetVolume(_)
            | command::Type::PlayResume(_)
            | command::Type::Pause(_)
            | command::Type::SkipNext(_)
            | command::Type::SkipPrevious(_)
            | command::Type::SeekToPos(_) => perms.can_use_controls,
            command::Type::Kick(_) | command::Type::Ban(_) => perms.can_manage_users,
            command::Type::DeleteRole(_)
            | command::Type::CreateRole(_)
            | command::Type::RenameRole(_) => perms.can_manage_users && perms.can_add_moderator,
        }
    }

    async fn get_spotify_handler(&self) -> Result<Spotify, command_response::Type> {
        let guard = self.sharify_state.read().await;

        let room = guard
            .get_room(&self.room_id)
            .ok_or(command_response::Type::RoomError(
                RoomError::RoomNotFound.into(),
            ))?;

        Ok(room.spotify_handler.clone())
    }
}

#[async_trait]
impl Commands for Command {
    type T = command_response::Type;
    type Output = Result<Option<Self::T>, Self::T>;

    async fn get_room(self) -> Self::Output {
        let guard = self.sharify_state.read().await;

        let room = guard
            .get_room(&self.room_id)
            .ok_or(Self::T::RoomError(RoomError::RoomNotFound.into()))?
            .clone();

        Ok(Some(Self::T::Room(room.into())))
    }

    async fn search(self, name: String) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        let tracks = spotify
            .search_track(name)
            .await
            .map_err(Into::<Self::T>::into)?;

        Ok(Some(Self::T::SpotifySearchResult(tracks.into())))
    }

    async fn add_to_queue(self, track: command::AddTrackToQueue) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .add_track_to_queue(track.track_id)
            .await
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn set_volume(self, percentage: u8) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .set_volume(percentage)
            .await
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn play_resume(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify.play_resume().await.map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn pause(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify.pause().await.map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn skip_next(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify.skip_next().await.map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn skip_previous(self) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .skip_previous()
            .await
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn seek_to_pos(self, pos: u64) -> Self::Output {
        let spotify = self.get_spotify_handler().await?;

        spotify
            .seek_to_ms(pos)
            .await
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn kick(self, opts: command::Kick) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        guard
            .kick_user(self.room_id, &self.user_id, &opts.user_id, opts.reason)
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn ban(self, opts: command::Ban) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        guard
            .ban_user(self.room_id, &self.user_id, &opts.user_id, opts.reason)
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn leave_room(self) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        guard
            .leave_room(self.room_id, self.user_id)
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn create_role(self, opts: command::CreateRole) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        let room = guard
            .get_room_mut(&self.room_id)
            .ok_or(Self::T::RoomError(RoomError::RoomNotFound.into()))?;

        room.role_manager
            .add_role(
                opts.name,
                opts.permissions
                    .ok_or(Self::T::GenericError(
                        "Permissions missing from request".into(),
                    ))?
                    .into(),
            )
            .map_err(Into::<Self::T>::into)?;

        Ok(None)
    }

    async fn rename_role(self, opts: command::RenameRole) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        let room = guard
            .get_room_mut(&self.room_id)
            .ok_or(Self::T::RoomError(RoomError::RoomNotFound.into()))?;

        let role_id = Uuid::from_slice(&opts.role_id[..16])
            .map_err(|err| Self::T::GenericError(format!("Failed to read role_id {err}")))?;

        let role = room
            .role_manager
            .get_role_by_id(&role_id)
            .ok_or(Self::T::RoomError(RoomError::RoleNotFound.into()))?;

        room.role_manager
            .edit_role(role_id, opts.name, role.permissions);

        Ok(None)
    }

    async fn delete_role(self, id: Vec<u8>) -> Self::Output {
        let mut guard = self.sharify_state.write().await;

        let room = guard
            .get_room_mut(&self.room_id)
            .ok_or(Self::T::RoomError(RoomError::RoomNotFound.into()))?;

        let role_id = Uuid::from_slice(&id[..16])
            .map_err(|err| Self::T::GenericError(format!("Failed to read role_id {err}")))?;

        room.role_manager.delete_role(role_id);

        Ok(None)
    }
}
