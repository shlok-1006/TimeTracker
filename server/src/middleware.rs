//! Auth middleware and authorization guards.
//!
//! Flow (CLAUDE.md backend architecture):
//!   1. `auth_middleware` validates the `Authorization: Bearer <jwt>` header,
//!      decodes the claims, and attaches an `AuthUser` to the request extensions.
//!   2. Handlers extract `AuthUser` (any authenticated user) or one of the guard
//!      extractors (`RequireEmployee` / `RequireStaff` / `RequireHr`) which
//!      enforce the role and return `403 Forbidden` on mismatch.

use axum::{
    async_trait,
    extract::{FromRequestParts, Request, State},
    http::{header::AUTHORIZATION, request::Parts},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

use crate::error::AppError;
use crate::jwt::Claims;
use crate::role::UserRole;
use crate::state::AppState;

/// The authenticated principal, derived from a verified JWT.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub role: UserRole,
    pub team: Option<Uuid>,
}

impl AuthUser {
    fn from_claims(c: Claims) -> Result<Self, AppError> {
        let id = c.sub.parse::<Uuid>().map_err(|_| AppError::Unauthorized)?;
        let team = match c.team {
            Some(t) => Some(t.parse::<Uuid>().map_err(|_| AppError::Unauthorized)?),
            None => None,
        };
        Ok(Self {
            id,
            role: c.role,
            team,
        })
    }
}

/// Validate the bearer token and attach `AuthUser` to the request.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = bearer_token(&req)?;
    let claims = state.jwt.verify(token)?;
    let user = AuthUser::from_claims(claims)?;
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}

fn bearer_token(req: &Request) -> Result<&str, AppError> {
    let value = req
        .headers()
        .get(AUTHORIZATION)
        .ok_or(AppError::Unauthorized)?
        .to_str()
        .map_err(|_| AppError::Unauthorized)?;
    value.strip_prefix("Bearer ").ok_or(AppError::Unauthorized)
}

// ---- Authorization guard predicates ----

/// Allow only employees (desktop app).
pub fn require_employee(role: UserRole) -> Result<(), AppError> {
    ensure(role == UserRole::Employee)
}

/// Allow admin-dashboard roles (HR or project manager).
pub fn require_staff(role: UserRole) -> Result<(), AppError> {
    ensure(role.is_dashboard())
}

/// Allow HR **or above** — i.e. HR and admin.
///
/// `at_least` rather than `== Hr`: an admin sits above HR precisely so it can do everything HR
/// can, and an equality test here would have locked the top of the hierarchy out of most of the
/// system. Every tier added above HR inherits these routes for free, which is the behaviour you
/// want from a rank.
pub fn require_hr(role: UserRole) -> Result<(), AppError> {
    ensure(role.at_least(UserRole::Hr))
}

/// Allow only `admin` — the seat above HR.
///
/// Reserved for the things HR must not do to itself: removing an HR account, and creating another
/// admin. Everything else HR can do, admin can do through `require_hr` above.
pub fn require_admin(role: UserRole) -> Result<(), AppError> {
    ensure(role == UserRole::Admin)
}

fn ensure(ok: bool) -> Result<(), AppError> {
    if ok {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

// ---- Extractors ----

/// Extract the authenticated user (set by `auth_middleware`). Yields
/// `401 Unauthorized` if the middleware was not applied / token was absent.
#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or(AppError::Unauthorized)
    }
}

/// Generates a guard extractor tuple-struct that enforces a role predicate.
macro_rules! guard_extractor {
    ($name:ident, $check:path) => {
        #[doc = concat!("Guard extractor enforcing `", stringify!($check), "`.")]
        pub struct $name(pub AuthUser);

        #[async_trait]
        impl<S> FromRequestParts<S> for $name
        where
            S: Send + Sync,
        {
            type Rejection = AppError;

            async fn from_request_parts(
                parts: &mut Parts,
                state: &S,
            ) -> Result<Self, Self::Rejection> {
                let user = AuthUser::from_request_parts(parts, state).await?;
                $check(user.role)?;
                Ok(Self(user))
            }
        }
    };
}

guard_extractor!(RequireEmployee, require_employee);
guard_extractor!(RequireStaff, require_staff);
guard_extractor!(RequireHr, require_hr);
guard_extractor!(RequireAdmin, require_admin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn employee_guard() {
        assert!(require_employee(UserRole::Employee).is_ok());
        assert!(require_employee(UserRole::Hr).is_err());
        assert!(require_employee(UserRole::ProjectManager).is_err());
    }

    #[test]
    fn admin_guard_allows_hr_and_pm() {
        assert!(require_staff(UserRole::Hr).is_ok());
        assert!(require_staff(UserRole::ProjectManager).is_ok());
        assert!(require_staff(UserRole::Employee).is_err());
    }

    #[test]
    fn hr_guard() {
        assert!(require_hr(UserRole::Hr).is_ok());
        assert!(require_hr(UserRole::ProjectManager).is_err());
        assert!(require_hr(UserRole::Employee).is_err());
    }
}
