use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub enum RoleError {
    NameAlreadyExists,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Role hierarchy is: Most powerful role first, then lower, then lower...
pub struct RoleManager(Vec<Role>);

impl RoleManager {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub(crate) fn new_from(roles: Vec<Role>) -> Self {
        Self(roles)
    }

    pub fn add_role(&mut self, name: String, permissions: RolePermission) -> Result<(), RoleError> {
        if self.0.iter().any(|role| role.name == name) {
            return Err(RoleError::NameAlreadyExists);
        }

        self.0.push(Role {
            id: Uuid::now_v7(),
            name,
            permissions,
        });

        self.sort();

        Ok(())
    }

    pub fn remove_role(&mut self, id: Uuid) {
        self.0.retain(|role| role.id == id);
    }

    pub fn edit_role(&mut self, id: Uuid, name: String, permissions: RolePermission) {
        for role in self.0.iter_mut() {
            if role.id != id {
                continue;
            }

            role.name = name;
            role.permissions = permissions;
            break;
        }
    }

    pub fn get_role_by_name(&self, name: &str) -> Option<&Role> {
        self.0.iter().find(|role| role.name == name)
    }

    pub fn get_role_by_id(&self, id: &Uuid) -> Option<&Role> {
        self.0.iter().find(|role| &role.id == id)
    }

    pub fn swap_roles(&mut self, idx1: usize, idx2: usize) {
        if idx1 >= self.0.len() || idx2 >= self.0.len() {
            return;
        }

        self.0.swap(idx1, idx2);
    }

    pub fn get_roles(&self) -> &Vec<Role> {
        &self.0
    }

    pub fn into_inner(self) -> Vec<Role> {
        self.0
    }

    fn sort(&mut self) {
        self.0.sort();
        self.0.reverse();
    }
}

impl IntoIterator for RoleManager {
    type Item = Role;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct RolePermission {
    pub can_use_controls: bool,
    /// Only users below
    pub can_manage_users: bool,
    pub can_add_song: bool,
    /// Moderator manager / Admin
    pub can_add_moderator: bool,
    /// Owner(s)
    pub can_manage_room: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Role {
    pub id: Uuid,
    pub name: String,
    pub permissions: RolePermission,
}

impl Role {
    pub fn new_guest() -> Self {
        Self {
            id: Uuid::now_v7(),
            name: "Guest".into(),
            permissions: RolePermission {
                can_use_controls: false,
                can_manage_users: false,
                can_add_song: false,
                can_add_moderator: false,
                can_manage_room: false,
            },
        }
    }

    pub fn new_vip() -> Self {
        Self {
            id: Uuid::now_v7(),
            name: "VIP".into(),
            permissions: RolePermission {
                can_use_controls: false,
                can_manage_users: false,
                can_add_song: true,
                can_add_moderator: false,
                can_manage_room: false,
            },
        }
    }

    pub fn new_moderator() -> Self {
        Self {
            id: Uuid::now_v7(),
            name: "Moderator".into(),
            permissions: RolePermission {
                can_use_controls: true,
                can_manage_users: true,
                can_add_song: true,
                can_add_moderator: false,
                can_manage_room: false,
            },
        }
    }

    pub fn new_admin() -> Self {
        Self {
            id: Uuid::now_v7(),
            name: "Admin".into(),
            permissions: RolePermission {
                can_use_controls: true,
                can_manage_users: true,
                can_add_song: true,
                can_add_moderator: true,
                can_manage_room: false,
            },
        }
    }

    pub fn new_owner() -> Self {
        Self {
            id: Uuid::now_v7(),
            name: "Owner".into(),
            permissions: RolePermission {
                can_use_controls: true,
                can_manage_users: true,
                can_add_song: true,
                can_add_moderator: true,
                can_manage_room: true,
            },
        }
    }
}

impl Default for RoleManager {
    fn default() -> Self {
        Self(Vec::from([
            Role::new_owner(),
            Role::new_admin(),
            Role::new_moderator(),
            Role::new_vip(),
            Role::new_guest(),
        ]))
    }
}

// Get rid of the warning for the * 1 which is nice for consistency
#[allow(clippy::identity_op)]
impl From<&Role> for u8 {
    fn from(role: &Role) -> Self {
        (role.permissions.can_add_song as u8) * 1
            + (role.permissions.can_use_controls as u8) * 2
            + (role.permissions.can_manage_users as u8) * 3
            + (role.permissions.can_add_moderator as u8) * 4
            + (role.permissions.can_manage_room as u8) * 5
    }
}

impl Eq for Role {}

impl PartialEq for Role {
    fn eq(&self, other: &Self) -> bool {
        u8::from(self) == u8::from(other)
    }
}

impl Ord for Role {
    fn cmp(&self, other: &Self) -> Ordering {
        u8::from(self).cmp(&other.into())
    }
}

impl PartialOrd for Role {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
