use std::collections::{HashMap, HashSet, VecDeque};

use rand::distr::Alphanumeric;
use rand::{Rng, rng};
use uuid::Uuid;

use super::role::*;
use super::room::*;
use super::room_metadata::*;
use super::utils::*;

#[derive(Debug, Default)]
pub struct RoomManager {
    active_rooms: HashMap<RoomID, Room>,
    user_ids: HashSet<RoomUserID>,
}

impl RoomManager {
    pub fn create_room(
        &mut self,
        user_id: RoomUserID,
        username: String,
        name: String,
        creds: CredentialsInput,
    ) -> Result<Room, RoomError> {
        if self.user_id_exists(&user_id) {
            return Err(RoomError::UserIDExists);
        }

        let id = Uuid::now_v7();
        let role_manager = RoleManager::default();

        self.active_rooms.insert(
            id,
            Room {
                id,
                users: Vec::from([RoomUser {
                    id: user_id,
                    username: username.clone(),
                    role_id: role_manager.get_roles()[0].id,
                    is_connected: false,
                }]),
                role_manager,
                name: name.clone(),
                password: rng()
                    .sample_iter(&Alphanumeric)
                    .take(0x10)
                    .map(char::from)
                    .collect::<String>(),
                logs: VecDeque::with_capacity(MAX_LOGS_LEN),
                banned_users: Vec::new(),
                tracks_queue: VecDeque::with_capacity(MAX_TRACKS_QUEUE_LEN),
                max_users: MAX_USERS,
                metadata: RoomMetadata::new(creds.into()),
            },
        );

        debug!("[{}] Room {} created", id, name);

        let Some(room) = self.active_rooms.get(&id) else {
            error!(
                "Unexpected error: Room not created user: {}, name: {}, active rooms len: {} cap: {}",
                username,
                name,
                self.active_rooms.len(),
                self.active_rooms.capacity(),
            );

            return Err(RoomError::RoomCreationFail);
        };

        for user in room.users.iter() {
            self.user_ids.insert(user.id.clone());
        }

        Ok(room.to_owned())
    }

    // If there's a user_id, it means that a user initiated the request
    // but if there is none, it means that the room self-destructed for inactivity
    pub fn delete_room(
        &mut self,
        room_id: RoomID,
        _user_id: Option<RoomUserID>,
    ) -> Result<(), RoomError> {
        let room = self.get_room(&room_id).ok_or(RoomError::RoomNotFound)?;

        if let Some(user_id) = _user_id {
            let user = room
                .users
                .iter()
                .find(|user| user.id == user_id)
                .ok_or(RoomError::RoomUserNotFound)?;

            let role = room
                .role_manager
                .get_role_by_id(&user.role_id)
                .ok_or(RoomError::RoleNotFound)?;

            if !role.permissions.can_manage_room {
                error!(
                    "User ID {} tried to delete room ID {} while not being having permissions ({:#?})",
                    user_id, room_id, role
                );

                return Err(RoomError::Unauthorized);
            }

            debug!(
                "[{}] User ID {} is deleting '{}' room",
                room_id, user_id, room.name
            );
        } else {
            debug!("Deleting room ID {room_id} automatically for inactivity");
        }

        let users = room.users.clone();
        let _ = room;

        for user in users {
            self.user_ids.remove(&user.id);
        }

        self.active_rooms.remove(&room_id);

        Ok(())
    }

    pub fn set_ws_user_state(
        &mut self,
        room_id: RoomID,
        user_id: &RoomUserID,
        is_connected: bool,
    ) -> Result<(), RoomError> {
        let room = self.get_room_mut(&room_id).ok_or(RoomError::RoomNotFound)?;

        let user = room
            .users
            .iter_mut()
            .find(|c| &c.id == user_id)
            .ok_or(RoomError::RoomUserNotFound)?;

        user.is_connected = is_connected;

        Ok(())
    }

    pub fn get_room(&self, room_id: &RoomID) -> Option<&Room> {
        let room = self.active_rooms.get(room_id);

        if room.is_none() {
            error!("Cannot find room id: {}", room_id);

            return None;
        }

        room
    }

    pub fn get_room_mut(&mut self, room_id: &RoomID) -> Option<&mut Room> {
        let room = self.active_rooms.get_mut(room_id);

        if room.is_none() {
            error!("Cannot find room id: {room_id}");

            return None;
        }

        room
    }

    pub fn get_room_for_user_id(&self, user_id: RoomUserID) -> Option<&Room> {
        self.active_rooms
            .values()
            .find(|&p| p.users.iter().any(|user| user.id == user_id))
    }

    pub fn add_track_to_queue(
        &mut self,
        id: RoomID,
        user_id: RoomUserID,
        track_id: String,
        track_name: String,
        track_duration: u32,
    ) -> Result<(), RoomError> {
        let room = self.get_room_mut(&id).ok_or(RoomError::RoomNotFound)?;

        let user = room
            .users
            .iter()
            .find(|c| c.id == user_id)
            .ok_or(RoomError::RoomUserNotFound)?;

        room.tracks_queue.push_back(RoomTrack {
            track_id,
            user_id,
            track_name: track_name.clone(),
            track_duration,
        });

        debug!(
            "{} added {} to room {} {}",
            user.username, track_name, room.name, id
        );

        Ok(())
    }

    /// Sort of fail-free fn that can be ran each time Spotify current playback is fetched
    pub fn remove_track_from_queue(
        &mut self,
        id: RoomID,
        track_id: String,
    ) -> Result<(), RoomError> {
        let room = self.get_room_mut(&id).ok_or(RoomError::RoomNotFound)?;

        if room
            .tracks_queue
            .front()
            .is_some_and(|t| t.track_id == track_id)
        {
            let track = room.tracks_queue.pop_front();

            debug!(
                "Removed track {:?} from room ID {} queue",
                track.map(|t| t.track_name),
                room.id
            );
        }

        Ok(())
    }

    pub fn kick_user(
        &mut self,
        room_id: RoomID,
        author_id: &RoomUserID,
        user_id: &RoomUserID,
        reason: String,
    ) -> Result<(), RoomError> {
        let room = self.get_room_mut(&room_id).ok_or(RoomError::RoomNotFound)?;

        // TODO: These are considered unrecoverable errors but at the Room' scope, not the app's
        // So destroy the room instead of crashing the app
        let Some(author) = room.users.iter().find(|c| c.id == *author_id).cloned() else {
            error!(
                "Unexpected error: Kick attempt from author id {author_id} that's not in the room id {room_id}"
            );
            dbg!(room);

            return Err(RoomError::Unreachable);
        };
        let Some(user) = room.users.iter().find(|c| c.id == *user_id).cloned() else {
            error!(
                "Unexpected error: Attempt to kick a user id {user_id} that's not in the room id {room_id}"
            );
            dbg!(room);

            return Err(RoomError::Unreachable);
        };

        room.users.retain(|c| c.id != *user_id);

        self.user_ids.remove(&user.id);

        self.append_log(
            room_id,
            Log::new(
                LogType::Kick,
                format!(
                    "User {} kicked {} from the room for: {}",
                    author.username, user.username, reason
                ),
            ),
        )?;

        Ok(())
    }

    pub fn ban_user(
        &mut self,
        room_id: RoomID,
        author_id: &RoomUserID,
        user_id: &RoomUserID,
        reason: String,
    ) -> Result<(), RoomError> {
        let room = self.get_room_mut(&room_id).ok_or(RoomError::RoomNotFound)?;

        // TODO: These are considered unrecoverable errors but at the Room' scope, not the app's
        // So destroy the room instead of crashing the app
        let Some(author) = room.users.iter().find(|c| c.id == *author_id).cloned() else {
            error!(
                "Unexpected error: Ban attempt from author id {author_id} that's not in the room id {room_id}"
            );

            return Err(RoomError::Unreachable);
        };
        let Some(user) = room.users.iter().find(|c| c.id == *user_id).cloned() else {
            error!(
                "Unexpected error: Attempt to ban a user id {user_id} that's not in the room id {room_id}"
            );

            return Err(RoomError::Unreachable);
        };

        room.users.retain(|c| c.id != *user_id);

        room.banned_users.push(user_id.clone());

        self.user_ids.remove(&user.id);

        self.append_log(
            room_id,
            Log::new(
                LogType::Ban,
                format!(
                    "User {} banned {} from the room for: {}",
                    author.username, user.username, reason
                ),
            ),
        )?;

        Ok(())
    }

    pub fn join_room(
        &mut self,
        room_id: RoomID,
        username: String,
        user_id: RoomUserID,
    ) -> Result<Room, RoomError> {
        if self.user_id_exists(&user_id) {
            error!(
                "Error: user ID (approx email: {}) is already in use",
                decode_user_email(&user_id)
            );

            return Err(RoomError::UserIDExists);
        }

        let room = self.get_room_mut(&room_id).ok_or(RoomError::RoomNotFound)?;

        if room.banned_users.contains(&user_id) {
            return Err(RoomError::UserBanned);
        }

        if room.users.len() == room.max_users {
            return Err(RoomError::RoomFull);
        }

        let role = match room.role_manager.get_roles().last().cloned() {
            Some(role) => role,
            None => {
                let guest = Role::new_guest();
                let _ = room
                    .role_manager
                    .add_role(guest.name.clone(), guest.permissions);

                guest
            }
        };

        room.users.push(RoomUser {
            id: user_id.clone(),
            role_id: role.id,
            username: username.clone(),
            is_connected: false,
        });

        let room = room.clone();

        debug!("[{}] Added {} to Room {}", room_id, username, room.name);

        self.user_ids.insert(user_id);

        Ok(room)
    }

    pub fn leave_room(&mut self, room_id: RoomID, user_id: RoomUserID) -> Result<(), RoomError> {
        if self.is_user_an_owner_and_alone(room_id, &user_id)? {
            return self.delete_room(room_id, Some(user_id));
        }

        let room = self.get_room_mut(&room_id).ok_or(RoomError::RoomNotFound)?;

        let user = room
            .users
            .iter()
            .find(|c| c.id == user_id)
            .cloned()
            .ok_or(RoomError::RoomUserNotFound)?;

        room.users.retain(|c| c.id != user_id);

        debug!(
            "Removed {} from room {} {}",
            user.username, room.name, room_id
        );

        self.user_ids.remove(&user.id);

        Ok(())
    }

    // FIXME rework
    // pub fn promote_user(
    //     &mut self,
    //     room_id: RoomID,
    //     mod_id: &RoomUserID,
    //     target_id: &RoomUserID,
    // ) -> Result<(), RoomError> {
    //     let room = self.get_room_mut(&room_id)?;
    //     let users = &room.users;
    //     let user = users.iter().find(|c| c.id == *target_id);
    //     let moderator = users.iter().find(|c| c.id == *mod_id);
    //
    //     if user.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find user ID {target_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let user = user.unwrap().clone();
    //
    //     if moderator.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find moderator user ID {mod_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let moderator = moderator.unwrap().clone();
    //
    //     if matches!(moderator.privileges.cmp(&user.privileges), Less | Equal) {
    //         return Err(RoomError::new(
    //             "You don't have privileges to do that".into(),
    //         ));
    //     }
    //
    //     if Privileges::try_from(user.privileges + 1).is_err()
    //         || *Privileges::try_from(user.privileges).unwrap() + 1 == *Privileges::Owner
    //     {
    //         return Err(RoomError::new(
    //             "Unexpected error: Cannot promote user to Owner or above the MAX privilege"
    //                 .into(),
    //         ));
    //     }
    //
    //     let _ = users;
    //
    //     room.users.iter_mut().for_each(|c| {
    //         if c.id == user.id {
    //             c.privileges += 1
    //         }
    //     });
    //
    //     debug!(
    //         "Mod ID {} changed User ID {} on Room ID {} to a {:?}",
    //         moderator.id,
    //         user.id,
    //         room.id,
    //         Privileges::try_from(user.privileges).unwrap()
    //     );
    //
    //     Ok(())
    // }

    // FIXME rework
    // pub fn demote_user(
    //     &mut self,
    //     room_id: RoomID,
    //     mod_id: &RoomUserID,
    //     target_id: &RoomUserID,
    // ) -> Result<(), RoomError> {
    //     let room = self.get_room_mut(&room_id)?;
    //     let users = &room.users;
    //     let user = users.iter().find(|c| c.id == *target_id);
    //     let moderator = users.iter().find(|c| c.id == *mod_id);
    //
    //     if user.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find user ID {target_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let user = user.unwrap().clone();
    //
    //     if moderator.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find moderator user ID {mod_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let moderator = moderator.unwrap().clone();
    //
    //     if matches!(moderator.privileges.cmp(&user.privileges), Less | Equal) {
    //         return Err(RoomError::new(
    //             "You don't have privileges to do that".into(),
    //         ));
    //     }
    //
    //     if Privileges::try_from(user.privileges - 1).is_err() {
    //         return Err(RoomError::new(
    //             "Unexpected error: Cannot demote user below the MIN privilege".into(),
    //         ));
    //     }
    //
    //     let _ = users;
    //
    //     room.users.iter_mut().for_each(|c| {
    //         if c.id == user.id {
    //             c.privileges -= 1
    //         }
    //     });
    //
    //     debug!(
    //         "Mod ID {} changed User ID {} on Room ID {} to a {:?}",
    //         moderator.id,
    //         user.id,
    //         room.id,
    //         Privileges::try_from(user.privileges).unwrap()
    //     );
    //
    //     Ok(())
    // }

    pub fn change_username(
        &mut self,
        id: RoomID,
        user_id: RoomUserID,
        username: String,
    ) -> Result<(), RoomError> {
        let room = self.get_room_mut(&id).ok_or(RoomError::RoomNotFound)?;

        let user = room
            .users
            .iter_mut()
            .find(|c| c.id == user_id)
            .ok_or(RoomError::RoomUserNotFound)?;

        user.username.clone_from(&username);

        Ok(())
    }

    /// Returns whether a user is an owner/room manager and if s.he is alone to control the room
    pub fn is_user_an_owner_and_alone(
        &self,
        room_id: RoomID,
        user_id: &RoomUserID,
    ) -> Result<bool, RoomError> {
        let room = self
            .active_rooms
            .get(&room_id)
            .ok_or(RoomError::RoomNotFound)?;

        let user = room
            .users
            .iter()
            .find(|&c| c.id == *user_id)
            .cloned()
            .ok_or(RoomError::RoomUserNotFound)?;

        let Some(role) = room.role_manager.get_role_by_id(&user.role_id) else {
            error!(
                "Cannot find role ID: {} in room ID: {room_id}, roles: {:?}",
                user.role_id,
                room.role_manager.get_roles()
            );

            return Err(RoomError::RoleNotFound);
        };

        // If role allows to manage room (most likely owner or one of them) and if there is nobody
        // else that can manage the room
        Ok(role.permissions.can_manage_room
            && room
                .users
                .iter()
                .filter(|c| {
                    c.role_id == role.id
                        || room
                            .role_manager
                            .get_role_by_id(&c.role_id)
                            .is_some_and(|r| r.permissions.can_manage_room)
                })
                .count()
                <= 1)
    }

    pub fn user_id_exists(&self, user_id: &RoomUserID) -> bool {
        self.user_ids.contains(user_id)
    }

    pub fn append_log(&mut self, room_id: RoomID, log: Log) -> Result<(), RoomError> {
        let room = self.get_room_mut(&room_id).ok_or(RoomError::RoomNotFound)?;

        if room.logs.len() >= MAX_LOGS_LEN {
            room.logs.pop_front();
        }

        room.logs.push_back(log);

        Ok(())
    }
}
