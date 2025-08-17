use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use actix_web::cookie::time::{ext::InstantExt, Duration};
use rand::distr::Alphanumeric;
use rand::{rng, Rng};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::role::{Role, RoleManager};
use super::spotify::{Spotify, SpotifyTokens, Timestamp};
use super::utils::decode_user_email;

const MAX_CLIENTS: u32 = 15;
const MAX_LOGS_LEN: usize = 25;
pub const INACTIVE_PARTY_MINS: u32 = 5;

// email / uuid allowed chars
pub(super) const MIN_EMAIL_CHAR: char = '-';
pub(super) const MAX_EMAIL_CHAR: char = 'z';

pub type RoomID = Uuid;
pub type RoomClientID = String;

#[derive(Clone, Debug, Serialize)]
pub struct Room {
    pub id: RoomID,
    pub name: String,
    pub password: String,
    pub clients: Vec<RoomClient>,
    pub banned_clients: Vec<RoomClientID>,
    /// Role hierarchy is: Most powerful role first, then less powerfull, then less...
    pub role_manager: RoleManager,
    // pub current_device: Option<SpotifyApi.UserDevice>,
    pub tracks_queue: Vec<RoomTrack>,
    pub max_clients: u32,
    // TODO: Add log on every action
    /// Last 25 logs: Ban, Kick, Song added... (25 for memory purposes)
    pub logs: VecDeque<Log>,
    #[serde(skip)]
    pub inactive_for: Option<Instant>,
    #[serde(skip)]
    pub spotify_handler: Spotify,
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
    pub expires_in: Timestamp,
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
    pub client_id: RoomClientID,
    pub track_id: String,
    pub track_name: String,
    pub track_duration: u32,
    #[serde(skip)]
    pub last_checked: Instant,
}

#[derive(Clone, Debug, Serialize)]
pub struct RoomClient {
    pub id: RoomClientID,
    pub username: String,
    pub role_id: Uuid,
    pub is_connected: bool, // TODO: Handle this everywhere
}

impl PartialEq for RoomClient {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Debug, Default)]
pub struct RoomManager {
    pub active_rooms: HashMap<RoomID, Room>,
    pub client_ids: HashSet<RoomClientID>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RoomError {
    pub error: String,
}

impl RoomError {
    pub fn new(error: String) -> Self {
        Self { error }
    }
}

impl std::fmt::Display for RoomError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl RoomManager {
    pub fn create_room(
        &mut self,
        client_id: RoomClientID,
        username: String,
        name: String,
        creds: CredentialsInput,
    ) -> Result<Room, RoomError> {
        info!("{:?} {client_id}", self.client_ids);
        if self.client_id_exists(&client_id) {
            return Err(RoomError::new(String::from(
                "This username is already taken",
            )));
        }

        let id = Uuid::now_v7();
        let role_manager = RoleManager::default();

        self.active_rooms.insert(
            id,
            Room {
                id,
                clients: Vec::from([RoomClient {
                    id: client_id,
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
                banned_clients: Vec::new(),
                tracks_queue: Vec::new(),
                max_clients: MAX_CLIENTS,
                spotify_handler: Spotify::new(creds.into()),
                inactive_for: None,
            },
        );

        debug!("[{}] Room {} created", id, name);

        let Some(room) = self.active_rooms.get(&id) else {
            error!(
                "Unexpected error: Room not created client: {}, name: {}, active rooms len: {} cap: {}",
                username,
                name,
                self.active_rooms.len(),
                self.active_rooms.capacity(),
            );

            return Err(RoomError::new(
                "Unexpected error: Failed to create Room".into(),
            ));
        };

        for client in room.clients.iter() {
            self.client_ids.insert(client.id.clone());
        }

        Ok(room.to_owned())
    }

    // If there's a client_id, it means that a client initiated the request
    // but if there is none, it means that the room self-destructed for inactivity
    pub fn delete_room(
        &mut self,
        id: RoomID,
        _client_id: Option<RoomClientID>,
    ) -> Result<(), RoomError> {
        let room = self
            .get_room(&id)
            .ok_or(RoomError::new(format!("Room ID {id} not found")))?;

        if let Some(client_id) = _client_id {
            let Some(client) = room.clients.iter().find(|client| client.id == client_id) else {
                return Err(RoomError::new(
                    "Unexpected error: Client not found on that Room".into(),
                ));
            };

            let Some(role) = room.role_manager.get_role_by_id(&client.role_id) else {
                return Err(RoomError::new(
                    "Unexpected error: Client Role not found on that Room".into(),
                ));
            };

            if !role.permissions.can_manage_room {
                error!(
                    "Client ID {} tried to delete room ID {} while not being having permissions ({:#?})",
                    client_id, id, role
                );

                return Err(RoomError::new(
                    "Your role does not allow you to delete a room".into(),
                ));
            }

            debug!(
                "[{}] Client ID {} is deleting '{}' room",
                id, client_id, room.name
            );
        } else {
            debug!("Deleting room ID {id} automatically for inactivity");
        }

        let clients = room.clients.clone();
        let _ = room;

        for client in clients {
            self.client_ids.remove(&client.id);
        }

        self.active_rooms.remove(&id);

        Ok(())
    }

    pub fn set_ws_client_state(
        &mut self,
        room_id: RoomID,
        client_id: &RoomClientID,
        is_connected: bool,
    ) -> Result<(), RoomError> {
        let Some(room) = self.get_room_mut(&room_id) else {
            debug!("Cannot find room id: {room_id}");

            return Err(RoomError::new(format!("Room {room_id} not found")));
        };

        let Some(client) = room.clients.iter_mut().find(|c| &c.id == client_id) else {
            debug!("Cannot find client id {client_id} for room id: {room_id}");

            return Err(RoomError::new(format!(
                "Client {client_id} not found for room {room_id}"
            )));
        };

        client.is_connected = is_connected;

        Ok(())
    }

    pub fn get_room(&self, id: &RoomID) -> Option<&Room> {
        let room = self.active_rooms.get(&id);

        if room.is_none() {
            error!("Cannot find room id: {}", id);

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

    pub fn get_room_for_client_id(&self, client_id: RoomClientID) -> Option<&Room> {
        self.active_rooms
            .values()
            .find(|&p| p.clients.iter().any(|client| client.id == client_id))
    }

    pub fn add_track_to_queue(
        &mut self,
        id: RoomID,
        client_id: RoomClientID,
        track_id: String,
        track_name: String,
        track_duration: u32,
    ) -> Result<(), RoomError> {
        let Some(room) = self.get_room_mut(&id) else {
            error!("Cannot find room id: {id}");

            return Err(RoomError::new(
                "An error has occured while adding this track to the queue, Room not found".into(),
            ));
        };

        let Some(client) = room.clients.iter().find(|c| c.id == client_id) else {
            debug!("Cannot find client id: {client_id} on id: {id}");

            return Err(RoomError::new(
                "An error has occured while adding this track to the queue, Client not found on this room".into()
            ));
        };

        room.tracks_queue.push(RoomTrack {
            track_id,
            client_id,
            track_name: track_name.clone(),
            track_duration,
            last_checked: Instant::now(),
        });

        debug!(
            "{} added {} to room {} {}",
            client.username, track_name, room.name, id
        );

        Ok(())
    }

    pub fn remove_track_from_queue(
        &mut self,
        id: RoomID,
        track_id: String,
    ) -> Result<(), RoomError> {
        let Some(room) = self.get_room_mut(&id) else {
            error!("Cannot find room id: {id}");

            return Err(RoomError::new(
                "An error has occured while removing a track from queue: Room not found".into(),
            ));
        };

        if let Some(idx) = room
            .tracks_queue
            .iter()
            .position(|track| track.track_id == track_id)
        {
            let track = room.tracks_queue.get(idx).unwrap();

            // FIXME: Wtf was the logic behind that
            if track
                .last_checked
                .add_signed(Duration::new((track.track_duration / 2) as i64 / 1000, 0))
                > Instant::now()
            {
                debug!(
                    "Removed track {} from room ID's {} queue",
                    track.track_name, room.id
                );
                room.tracks_queue.remove(idx);
            }

            return Ok(());
        }

        Err(RoomError::new("Track not found in the queue".into()))
    }

    pub fn kick_client(
        &mut self,
        room_id: RoomID,
        author_id: &RoomClientID,
        client_id: &RoomClientID,
        reason: String,
    ) -> Result<(), RoomError> {
        let Some(room) = self.active_rooms.get_mut(&room_id) else {
            error!("Cannot find room id: {room_id}");

            return Err(RoomError::new("Cannot find room".into()));
        };

        let Some(author) = room.clients.iter().find(|c| c.id == *author_id).cloned() else {
            error!("Unexpected error: Kick attempt from author id {author_id} that's not in the room id {room_id}");

            return Err(RoomError::new(
                "Kick author is not in the room (anymore)".into(),
            ));
        };
        let Some(client) = room.clients.iter().find(|c| c.id == *client_id).cloned() else {
            error!("Unexpected error: Attempt to kick a client id {client_id} that's not in the room id {room_id}");

            return Err(RoomError::new(
                "Tried to kick a user that's not in the room (anymore)".into(),
            ));
        };

        room.clients.retain(|c| c.id != *client_id);

        self.client_ids.remove(&client.id);

        self.append_log(
            room_id,
            Log::new(
                LogType::Kick,
                format!(
                    "Client {} kicked {} from the room for: {}",
                    author.username, client.username, reason
                ),
            ),
        )?;

        Ok(())
    }

    pub fn ban_client(
        &mut self,
        room_id: RoomID,
        author_id: &RoomClientID,
        client_id: &RoomClientID,
        reason: String,
    ) -> Result<(), RoomError> {
        let Some(room) = self.active_rooms.get_mut(&room_id) else {
            error!("Cannot find room id: {room_id}");

            return Err(RoomError::new("Cannot find room".into()));
        };

        let Some(author) = room.clients.iter().find(|c| c.id == *author_id).cloned() else {
            error!("Unexpected error: Ban attempt from author id {author_id} that's not in the room id {room_id}");

            return Err(RoomError::new(
                "Ban author is not in the room (anymore)".into(),
            ));
        };
        let Some(client) = room.clients.iter().find(|c| c.id == *client_id).cloned() else {
            error!("Unexpected error: Attempt to ban a client id {client_id} that's not in the room id {room_id}");

            return Err(RoomError::new(
                "Tried to ban a user that's not in the room (anymore)".into(),
            ));
        };

        room.clients.retain(|c| c.id != *client_id);

        self.client_ids.remove(&client.id);

        room.banned_clients.push(client_id.clone());

        self.append_log(
            room_id,
            Log::new(
                LogType::Ban,
                format!(
                    "Client {} banned {} from the room for: {}",
                    author.username, client.username, reason
                ),
            ),
        )?;

        Ok(())
    }

    pub fn join_room(
        &mut self,
        room_id: RoomID,
        username: String,
        client_id: RoomClientID,
    ) -> Result<Room, RoomError> {
        if self.client_id_exists(&client_id) {
            return Err(RoomError::new(format!(
                "Error: user ID (approx email: {}) is already in use",
                decode_user_email(&client_id)
            )));
        }

        let Some(room) = self.active_rooms.get_mut(&room_id) else {
            debug!("Cannot find room id: {room_id}");

            return Err(RoomError::new(format!("Room [{}] not found", room_id)));
        };

        if room.banned_clients.contains(&client_id) {
            return Err(RoomError::new("You are banned from that Room".into()));
        }

        if room.clients.len() == room.max_clients as usize {
            return Err(RoomError::new(format!(
                "Room full, max clients: {}",
                room.max_clients
            )));
        }

        let role = match room.role_manager.get_roles().last().cloned() {
            Some(role) => role,
            None => {
                let guest = Role::new_guest();
                room.role_manager
                    .add_role(guest.name.clone(), guest.permissions);

                guest
            }
        };

        room.clients.push(RoomClient {
            id: client_id.clone(),
            role_id: role.id,
            username: username.clone(),
            is_connected: false,
        });

        debug!("[{}] Added {} to Room {}", room_id, username, room.name);

        self.client_ids.insert(client_id);

        Ok(room.to_owned())
    }

    pub fn leave_room(
        &mut self,
        room_id: RoomID,
        client_id: RoomClientID,
    ) -> Result<(), RoomError> {
        let Some(room) = self.active_rooms.get_mut(&room_id) else {
            error!("Cannot find room id: {room_id}");

            return Err(RoomError::new("Cannot find room".into()));
        };

        let client = room
            .clients
            .iter()
            .find(|c| c.id == client_id)
            .cloned()
            .ok_or(RoomError::new(format!(
                "Cannot find client ID {client_id} on room ID {room_id}"
            )))?;

        let Some(role) = room.role_manager.get_role_by_id(&client.role_id) else {
            error!(
                "Cannot find role ID: {} in room ID: {room_id}, roles: {:?}",
                client.role_id,
                room.role_manager.get_roles()
            );

            return Err(RoomError::new(
                "Cannot find client's role within the room".into(),
            ));
        };

        // If role allows to manage room (most likely owner or one of them) and if there is nobody
        // else that can manage the room
        if role.permissions.can_manage_room
            && room
                .clients
                .iter()
                .filter(|c| {
                    c.role_id == role.id
                        || room
                            .role_manager
                            .get_role_by_id(&c.role_id)
                            .is_some_and(|r| r.permissions.can_manage_room)
                })
                .count()
                <= 1
        {
            let room_id = room.id;
            let _ = room; // Implicit drop of the mut ref to allow the use of mut ref self below
            return self.delete_room(room_id, Some(client.id));
        }

        room.clients.retain(|c| c.id != client_id);

        self.client_ids.remove(&client.id);

        debug!(
            "Removed {} from room {} {}",
            client.username, room.name, room_id
        );

        Ok(())
    }

    // FIXME rework
    // pub fn promote_user(
    //     &mut self,
    //     room_id: RoomID,
    //     mod_id: &RoomClientID,
    //     target_id: &RoomClientID,
    // ) -> Result<(), RoomError> {
    //     let room = self.get_room_mut(&room_id)?;
    //     let clients = &room.clients;
    //     let client = clients.iter().find(|c| c.id == *target_id);
    //     let moderator = clients.iter().find(|c| c.id == *mod_id);
    //
    //     if client.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find client ID {target_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let client = client.unwrap().clone();
    //
    //     if moderator.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find moderator client ID {mod_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let moderator = moderator.unwrap().clone();
    //
    //     if matches!(moderator.privileges.cmp(&client.privileges), Less | Equal) {
    //         return Err(RoomError::new(
    //             "You don't have privileges to do that".into(),
    //         ));
    //     }
    //
    //     if Privileges::try_from(client.privileges + 1).is_err()
    //         || *Privileges::try_from(client.privileges).unwrap() + 1 == *Privileges::Owner
    //     {
    //         return Err(RoomError::new(
    //             "Unexpected error: Cannot promote client to Owner or above the MAX privilege"
    //                 .into(),
    //         ));
    //     }
    //
    //     let _ = clients;
    //
    //     room.clients.iter_mut().for_each(|c| {
    //         if c.id == client.id {
    //             c.privileges += 1
    //         }
    //     });
    //
    //     debug!(
    //         "Mod ID {} changed Client ID {} on Room ID {} to a {:?}",
    //         moderator.id,
    //         client.id,
    //         room.id,
    //         Privileges::try_from(client.privileges).unwrap()
    //     );
    //
    //     Ok(())
    // }

    // FIXME rework
    // pub fn demote_user(
    //     &mut self,
    //     room_id: RoomID,
    //     mod_id: &RoomClientID,
    //     target_id: &RoomClientID,
    // ) -> Result<(), RoomError> {
    //     let room = self.get_room_mut(&room_id)?;
    //     let clients = &room.clients;
    //     let client = clients.iter().find(|c| c.id == *target_id);
    //     let moderator = clients.iter().find(|c| c.id == *mod_id);
    //
    //     if client.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find client ID {target_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let client = client.unwrap().clone();
    //
    //     if moderator.is_none() {
    //         return Err(RoomError::new(format!(
    //             "Cannot find moderator client ID {mod_id} on room ID {room_id}"
    //         )));
    //     }
    //
    //     let moderator = moderator.unwrap().clone();
    //
    //     if matches!(moderator.privileges.cmp(&client.privileges), Less | Equal) {
    //         return Err(RoomError::new(
    //             "You don't have privileges to do that".into(),
    //         ));
    //     }
    //
    //     if Privileges::try_from(client.privileges - 1).is_err() {
    //         return Err(RoomError::new(
    //             "Unexpected error: Cannot demote client below the MIN privilege".into(),
    //         ));
    //     }
    //
    //     let _ = clients;
    //
    //     room.clients.iter_mut().for_each(|c| {
    //         if c.id == client.id {
    //             c.privileges -= 1
    //         }
    //     });
    //
    //     debug!(
    //         "Mod ID {} changed Client ID {} on Room ID {} to a {:?}",
    //         moderator.id,
    //         client.id,
    //         room.id,
    //         Privileges::try_from(client.privileges).unwrap()
    //     );
    //
    //     Ok(())
    // }

    pub fn change_username(
        &mut self,
        id: RoomID,
        client_id: RoomClientID,
        username: String,
    ) -> Result<(), RoomError> {
        let room = self
            .get_room_mut(&id)
            .ok_or(RoomError::new(format!("Room ID {id} not found")))?;

        let Some(client) = room.clients.iter_mut().find(|c| c.id == client_id) else {
            error!("Unexpected error: A Client newly named {username} tried to rename themselves on the Room ID {id} but the Client (ID {client_id}) doesn't exists within the room");

            return Err(RoomError::new(format!(
                "Unexpected error: Client id {client_id} not found on room id {id}"
            )));
        };

        client.username.clone_from(&username);

        Ok(())
    }

    pub fn client_id_exists(&self, client_id: &RoomClientID) -> bool {
        self.client_ids.contains(client_id)
    }

    pub fn append_log(&mut self, room_id: RoomID, log: Log) -> Result<(), RoomError> {
        let room = self
            .get_room_mut(&room_id)
            .ok_or(RoomError::new(format!("Room ID {room_id} not found")))?;

        if room.logs.len() >= MAX_LOGS_LEN {
            room.logs.pop_front();
        }

        room.logs.push_back(log);

        Ok(())
    }
}

impl Room {
    pub fn to_json(&self) -> Value {
        json!(self)
    }
}
