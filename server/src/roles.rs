//! Who may do what. Every account has one role, shared by requests, movies and TV:
//!
//! | Role | May |
//! | --- | --- |
//! | `owner` | everything, and see saved secrets; made at the first start, never removed |
//! | `admin` | everything: settings, accounts, approving requests, managing movies and TV |
//! | `requester` | look around and ask for movies and TV, within the account's quotas (auto-approved when set) |
//! | `viewer` | look around only (read-only) |

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use kit_server_core::{err, Account, Result};
use serde::Serialize;

/// An account's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The server's owner: an admin who may also see saved secrets.
    Owner,
    /// May change everything.
    Admin,
    /// May ask for movies and TV.
    Requester,
    /// May only look.
    Viewer,
}

impl Role {
    /// Every role, most rights first.
    pub const ALL: [Role; 4] = [Role::Owner, Role::Admin, Role::Requester, Role::Viewer];

    /// The role stored as `s`; an unknown value (the template's `user`) is a requester.
    pub fn parse(s: &str) -> Role {
        match s {
            "owner" => Role::Owner,
            "admin" => Role::Admin,
            "viewer" => Role::Viewer,
            _ => Role::Requester,
        }
    }

    /// The role a person may give an account (never `owner`), from its stored name; `None` when it is not one.
    pub fn assignable(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "requester" => Some(Role::Requester),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }

    /// How the role is stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Requester => "requester",
            Role::Viewer => "viewer",
        }
    }

    /// The role's name as people see it.
    pub fn label(self) -> &'static str {
        match self {
            Role::Owner => "Owner",
            Role::Admin => "Admin",
            Role::Requester => "Can request",
            Role::Viewer => "Read-only",
        }
    }

    /// What the role may do, in one plain sentence.
    pub fn description(self) -> &'static str {
        match self {
            Role::Owner => "Can do everything, including seeing saved passwords and keys.",
            Role::Admin => "Can do everything: settings, accounts, approving requests, movies and TV.",
            Role::Requester => "Can look around and ask for movies and TV, within their limits.",
            Role::Viewer => "Can look around but can't ask for or change anything.",
        }
    }

    /// Whether the role may change settings, accounts, movies and TV.
    pub fn is_admin(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }

    /// Whether the role may ask for movies and TV.
    pub fn can_request(self) -> bool {
        !matches!(self, Role::Viewer)
    }
}

/// The signed-in account of a request and its role; a handler that takes it answers 401 to anyone signed out.
#[derive(Clone, Debug)]
pub struct Me {
    /// the account as the kit sees it
    pub account: Account,
    /// the account's role
    pub role: Role,
}

impl Me {
    /// `Ok` for an owner or admin, else 403 with a plain sentence.
    pub fn admin(&self) -> Result<()> {
        if self.role.is_admin() {
            Ok(())
        } else {
            Err(err(403, "Only an admin can do this"))
        }
    }
}

impl FromRequestParts<crate::AppState> for Me {
    type Rejection = kit_server_core::Error;

    async fn from_request_parts(parts: &mut Parts, st: &crate::AppState) -> Result<Me> {
        let (account, role) = crate::auth::signed_in_with_role(st, parts)?;
        Ok(Me { account, role })
    }
}

#[cfg(test)]
mod tests {
    use super::Role;

    #[test]
    fn roles_round_trip_and_unknown_is_requester() {
        for r in Role::ALL {
            assert_eq!(Role::parse(r.as_str()), r);
        }
        assert_eq!(Role::parse("user"), Role::Requester);
        assert_eq!(Role::assignable("owner"), None);
        assert!(Role::Admin.is_admin() && !Role::Requester.is_admin());
        assert!(Role::Requester.can_request() && !Role::Viewer.can_request());
    }
}
