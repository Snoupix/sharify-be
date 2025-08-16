use std::sync::{Arc, RwLock};

use crate::proto::cmd::command;
use crate::proto::cmd::command_response;
use crate::proto::room::RoomTrack;
use crate::sharify::room::RoomManager;
use crate::sharify::room::{RoomClientID, RoomID};
use crate::sharify::spotify::Spotify;
use crate::sharify::websocket::SharifyWsManager;

trait Commands {
    type Output;

    fn search(&self, name: String) -> Self::Output;
    fn add_to_queue(&self, track: RoomTrack) -> Self::Output;
    fn set_volume(&self, percentage: u8) -> Self::Output;
    fn play_resume(&self) -> Self::Output;
    fn pause(&self) -> Self::Output;
    fn skip_next(&self) -> Self::Output;
    fn skip_previous(&self) -> Self::Output;
    fn seek_to_pos(&self) -> Self::Output;
    fn kick(&self, opts: command::Kick) -> Self::Output;
    fn ban(&self, opts: command::Ban) -> Self::Output;
    fn get_room(&self) -> Self::Output;
}

pub struct Command {
    sharify_state: Arc<RwLock<RoomManager>>,
    ws_manager: Arc<RwLock<SharifyWsManager>>,
    author_id: RoomClientID,
    room_id: RoomID,
}

impl Command {
    pub fn new(
        sharify_state: Arc<RwLock<RoomManager>>,
        ws_manager: Arc<RwLock<SharifyWsManager>>,
        author_id: RoomClientID,
        room_id: RoomID,
    ) -> Self {
        Self {
            sharify_state,
            ws_manager,
            author_id,
            room_id,
        }
    }

    pub fn process(&mut self, cmd_type: command::Type) -> command_response::Type {
        if !self.has_permission_to(&cmd_type) {
            return command_response::Type::Unauthorized(false);
        }

        match cmd_type {
            command::Type::Search(name) => self.search(name),
            command::Type::AddToQueue(room_track) => self.add_to_queue(room_track),
            command::Type::SetVolume(percentage) => self.set_volume(percentage as _),
            command::Type::PlayResume(_) => self.play_resume(),
            command::Type::Pause(_) => self.pause(),
            command::Type::SkipNext(_) => self.skip_next(),
            command::Type::SkipPrevious(_) => self.skip_previous(),
            command::Type::SeekToPos(_) => self.seek_to_pos(),
            command::Type::Kick(opts) => self.kick(opts),
            command::Type::Ban(opts) => self.ban(opts),
            command::Type::GetRoom(_) => self.get_room(),
        }
    }

    fn has_permission_to(&self, cmd_type: &command::Type) -> bool {
        let guard = self.sharify_state.read().unwrap();
        let Some(room) = guard.get_room(&self.room_id) else {
            return false;
        };
        let Some(client_role_id) = room.clients.iter().find_map(|client| {
            if client.id == self.author_id {
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

    fn get_spotify_handler(&self) -> Option<Spotify> {
        let guard = self.sharify_state.read().unwrap();
        let room = guard.get_room(&self.room_id)?;

        Some(room.spotify_handler.clone())
    }
}

impl Commands for Command {
    type Output = command_response::Type;

    fn search(&self, name: String) -> Self::Output {
        todo!()
    }

    fn add_to_queue(&self, track: RoomTrack) -> Self::Output {
        todo!()
    }

    fn set_volume(&self, percentage: u8) -> Self::Output {
        todo!()
    }

    fn play_resume(&self) -> Self::Output {
        todo!()
    }

    fn pause(&self) -> Self::Output {
        todo!()
    }

    fn skip_next(&self) -> Self::Output {
        todo!()
    }

    fn skip_previous(&self) -> Self::Output {
        todo!()
    }

    fn seek_to_pos(&self) -> Self::Output {
        todo!()
    }

    fn kick(&self, opts: command::Kick) -> Self::Output {
        todo!()
    }

    fn ban(&self, opts: command::Ban) -> Self::Output {
        todo!()
    }

    fn get_room(&self) -> Self::Output {
        let guard = self.sharify_state.read().unwrap();

        let room = guard.get_room(&self.room_id).unwrap().clone();

        command_response::Type::Room(room.into())
    }
}
