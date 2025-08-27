use uuid::Uuid;

use crate::proto;
use crate::sharify::role;

impl From<role::RoleError> for i32 {
    fn from(err: role::RoleError) -> Self {
        match err {
            role::RoleError::NameAlreadyExists => 0,
        }
    }
}

impl From<role::RoleError> for proto::cmd::command_response::Type {
    fn from(err: role::RoleError) -> Self {
        Self::RoleError(err.into())
    }
}

impl From<proto::role::RoleError> for role::RoleError {
    fn from(err: proto::role::RoleError) -> Self {
        match err {
            proto::role::RoleError::NameAlreadyExists => Self::NameAlreadyExists,
        }
    }
}

impl From<role::RoleError> for proto::role::RoleError {
    fn from(err: role::RoleError) -> Self {
        match err {
            role::RoleError::NameAlreadyExists => Self::NameAlreadyExists,
        }
    }
}

impl From<proto::role::RolePermission> for role::RolePermission {
    fn from(perm: proto::role::RolePermission) -> Self {
        Self {
            can_use_controls: perm.can_use_controls,
            can_manage_users: perm.can_manage_users,
            can_add_song: perm.can_add_song,
            can_add_moderator: perm.can_add_moderator,
            can_manage_room: perm.can_manage_room,
        }
    }
}

impl From<role::RolePermission> for proto::role::RolePermission {
    fn from(perm: role::RolePermission) -> Self {
        Self {
            can_use_controls: perm.can_use_controls,
            can_manage_users: perm.can_manage_users,
            can_add_song: perm.can_add_song,
            can_add_moderator: perm.can_add_moderator,
            can_manage_room: perm.can_manage_room,
        }
    }
}

impl From<proto::role::Role> for role::Role {
    fn from(role: proto::role::Role) -> Self {
        Self {
            id: Uuid::from_slice(&role.id[..16]).unwrap(),
            name: role.name,
            permissions: role.permissions.map(Into::into).unwrap(),
        }
    }
}

impl From<role::Role> for proto::role::Role {
    fn from(role: role::Role) -> Self {
        Self {
            id: role.id.into_bytes().into(),
            name: role.name,
            permissions: Some(role.permissions.into()),
        }
    }
}

impl From<proto::role::RoleManager> for role::RoleManager {
    fn from(role_manager: proto::role::RoleManager) -> Self {
        Self::new_from(role_manager.roles.into_iter().map(Into::into).collect())
    }
}

impl From<role::RoleManager> for proto::role::RoleManager {
    fn from(role_manager: role::RoleManager) -> Self {
        Self {
            roles: role_manager
                .into_inner()
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}
