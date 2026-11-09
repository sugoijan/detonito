use barbed::helix::{HelixError, TwitchTokenExchange};
use barbed::oauth::{OAuthStatePayload, TwitchTokenState};
use barbed::signing::{self, SigningError};
use detonito_protocol::AfkIdentity;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use worker::Url;

use crate::{join_base_path, normalize_base_path};

const OAUTH_STATE_TTL_MS: i64 = 10 * 60 * 1_000;
const AUTH_TOKEN_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
const AUTH_TOKEN_TTL_SECS: i64 = AUTH_TOKEN_TTL_MS / 1_000;
const TOKEN_REFRESH_SKEW_MS: i64 = 5 * 60 * 1_000;

pub const AUTH_COOKIE_NAME: &str = "detonito_auth";
pub const TWITCH_SCOPES: &[&str] = &[
    "chat:read",
    "user:read:chat",
    "moderator:manage:banned_users",
];

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("twitch client id is not configured")]
    MissingClientId,
    #[error("public url is not configured")]
    MissingPublicUrl,
    #[error("signed auth payload is expired")]
    Expired,
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error("failed to parse url: {0}")]
    Url(#[from] worker::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthState {
    pub return_to: Option<String>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

impl OAuthStatePayload for OAuthState {
    fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAuthClaims {
    pub identity: AfkIdentity,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedAuthCallback {
    pub auth: SignedAuthClaims,
    pub auth_token: String,
    pub redirect_url: Url,
}

pub fn build_twitch_authorize_url(
    client_id: &str,
    public_url: &str,
    return_to: Option<&str>,
    now_ms: i64,
    signing_secret: &str,
) -> Result<Url, AuthError> {
    if client_id.is_empty() {
        return Err(AuthError::MissingClientId);
    }
    if public_url.is_empty() {
        return Err(AuthError::MissingPublicUrl);
    }

    let state = OAuthState {
        return_to: sanitized_return_to_path(return_to),
        issued_at_ms: now_ms,
        expires_at_ms: now_ms + OAUTH_STATE_TTL_MS,
    };
    let redirect_uri = callback_url(public_url)?;
    let url = barbed::oauth::build_authorize_url(
        client_id,
        redirect_uri.as_str(),
        TWITCH_SCOPES,
        &state,
        signing_secret,
    )?;
    Url::parse(&url)
        .map_err(worker::Error::from)
        .map_err(AuthError::from)
}

pub fn verify_oauth_state(
    signing_secret: &str,
    token: &str,
    now_ms: i64,
) -> Result<OAuthState, AuthError> {
    barbed::oauth::verify_oauth_state(signing_secret, token, now_ms).map_err(AuthError::from)
}

pub fn sign_auth_token(
    signing_secret: &str,
    claims: &SignedAuthClaims,
) -> Result<String, AuthError> {
    sign_payload(signing_secret, claims)
}

pub fn verify_auth_token(
    signing_secret: &str,
    token: &str,
    now_ms: i64,
) -> Result<SignedAuthClaims, AuthError> {
    let claims: SignedAuthClaims = verify_signed_payload(signing_secret, token)?;
    if now_ms > claims.expires_at_ms {
        return Err(AuthError::Expired);
    }
    Ok(claims)
}

pub fn should_refresh_auth_token(claims: &SignedAuthClaims, now_ms: i64) -> bool {
    now_ms >= claims.expires_at_ms - TOKEN_REFRESH_SKEW_MS
}

pub fn refreshed_auth_claims(claims: &SignedAuthClaims, now_ms: i64) -> SignedAuthClaims {
    SignedAuthClaims {
        identity: claims.identity.clone(),
        issued_at_ms: now_ms,
        expires_at_ms: now_ms + AUTH_TOKEN_TTL_MS,
    }
}

pub fn refreshed_twitch_token_state(
    previous: &TwitchTokenState,
    mut exchange: TwitchTokenExchange,
    now_ms: i64,
) -> Result<TwitchTokenState, HelixError> {
    // Refresh responses may omit metadata; retain the existing session values.
    exchange.scope.get_or_insert_with(|| previous.scope.clone());
    exchange
        .token_type
        .get_or_insert_with(|| previous.token_type.clone());
    exchange.into_token_state(Some(&previous.refresh_token), now_ms)
}

pub fn complete_twitch_callback(
    public_url: &str,
    signing_secret: &str,
    state: OAuthState,
    identity: AfkIdentity,
    now_ms: i64,
) -> Result<CompletedAuthCallback, AuthError> {
    let auth = SignedAuthClaims {
        identity,
        issued_at_ms: now_ms,
        expires_at_ms: now_ms + AUTH_TOKEN_TTL_MS,
    };
    let auth_token = sign_auth_token(signing_secret, &auth)?;
    let redirect_url = callback_redirect_url(public_url, state.return_to.as_deref())?;
    Ok(CompletedAuthCallback {
        auth,
        auth_token,
        redirect_url,
    })
}

pub fn auth_token_from_authorization_header(value: &str) -> Option<&str> {
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
}

pub fn auth_token_from_cookie_header(value: &str) -> Option<&str> {
    value.split(';').find_map(|part| {
        let trimmed = part.trim();
        let (name, cookie_value) = trimmed.split_once('=')?;
        if name == AUTH_COOKIE_NAME && !cookie_value.is_empty() {
            Some(cookie_value)
        } else {
            None
        }
    })
}

pub fn auth_cookie_header(token: &str, cookie_path: &str, secure: bool) -> String {
    format!(
        "{AUTH_COOKIE_NAME}={token}; Path={cookie_path}; Max-Age={AUTH_TOKEN_TTL_SECS}; HttpOnly; SameSite=Lax{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn cleared_auth_cookie_header(cookie_path: &str, secure: bool) -> String {
    format!(
        "{AUTH_COOKIE_NAME}=; Path={cookie_path}; Max-Age=0; HttpOnly; SameSite=Lax{}",
        if secure { "; Secure" } else { "" }
    )
}

fn callback_url(public_url: &str) -> Result<Url, AuthError> {
    join_public_url(public_url, "/auth/twitch/callback").map_err(AuthError::from)
}

fn callback_redirect_url(public_url: &str, return_to: Option<&str>) -> Result<Url, AuthError> {
    let mut url = Url::parse(public_url).map_err(worker::Error::from)?;
    let base_path = url.path().to_string();
    let return_to = sanitized_return_to_path(return_to).unwrap_or_else(|| "/".to_string());
    let parsed =
        Url::parse(&format!("https://detonito.invalid{return_to}")).map_err(worker::Error::from)?;

    url.set_path(&join_base_path(&base_path, parsed.path()));
    url.set_query(parsed.query());
    url.set_fragment(None);
    Ok(url)
}

fn sanitized_return_to_path(return_to: Option<&str>) -> Option<String> {
    let return_to = return_to?.trim();
    if return_to.is_empty() {
        return Some("/".to_string());
    }
    if !return_to.starts_with('/') || return_to.starts_with("//") || return_to.contains('\\') {
        return None;
    }

    let parsed = Url::parse(&format!("https://detonito.invalid{return_to}")).ok()?;
    let mut normalized = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        normalized.push('?');
        normalized.push_str(query);
    }
    Some(normalized)
}

fn sign_payload<T: Serialize>(signing_secret: &str, value: &T) -> Result<String, AuthError> {
    signing::sign_payload(signing_secret, value).map_err(AuthError::from)
}

fn verify_signed_payload<T: DeserializeOwned>(
    signing_secret: &str,
    token: &str,
) -> Result<T, AuthError> {
    signing::verify_signed_payload(signing_secret, token).map_err(AuthError::from)
}

fn join_public_url(public_url: &str, path: &str) -> Result<Url, worker::Error> {
    let mut url = Url::parse(public_url).map_err(worker::Error::from)?;
    let base_path = normalize_base_path(url.path());
    url.set_path(&join_base_path(&base_path, path));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "detonito-secret";

    #[test]
    fn sanitized_return_to_rejects_external_paths() {
        assert_eq!(sanitized_return_to_path(Some("https://evil.example")), None);
        assert_eq!(sanitized_return_to_path(Some("//evil.example")), None);
        assert_eq!(
            sanitized_return_to_path(Some("/afk?mode=1")),
            Some("/afk?mode=1".into())
        );
    }

    #[test]
    fn auth_cookie_round_trips() {
        let header = auth_cookie_header("signed-token", "/", true);
        assert_eq!(auth_token_from_cookie_header(&header), Some("signed-token"));
    }

    #[test]
    fn auth_claims_refresh_extends_expiry() {
        let claims = SignedAuthClaims {
            identity: AfkIdentity::new("1", "tester", "Tester"),
            issued_at_ms: 10,
            expires_at_ms: 20,
        };
        let refreshed = refreshed_auth_claims(&claims, 15);
        assert_eq!(refreshed.identity.login, "tester");
        assert!(refreshed.expires_at_ms > claims.expires_at_ms);
    }

    #[test]
    fn twitch_refresh_preserves_omitted_session_metadata() {
        let previous = TwitchTokenState {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_in_seconds: Some(3_600),
            scope: vec!["moderator:manage:banned_users".into()],
            token_type: "Bearer".into(),
            linked_at_ms: 10,
        };
        let exchange = TwitchTokenExchange {
            access_token: "new-access".into(),
            refresh_token: None,
            expires_in: Some(7_200),
            scope: None,
            token_type: None,
        };

        let refreshed = refreshed_twitch_token_state(&previous, exchange, 20).unwrap();
        assert_eq!(
            refreshed,
            TwitchTokenState {
                access_token: "new-access".into(),
                expires_in_seconds: Some(7_200),
                linked_at_ms: 20,
                ..previous
            }
        );
    }

    #[test]
    fn twitch_refresh_applies_rotated_tokens_and_explicit_empty_scopes() {
        let previous = TwitchTokenState {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_in_seconds: Some(3_600),
            scope: vec!["moderator:manage:banned_users".into()],
            token_type: "Bearer".into(),
            linked_at_ms: 10,
        };
        let exchange = TwitchTokenExchange {
            access_token: "new-access".into(),
            refresh_token: Some("new-refresh".into()),
            expires_in: None,
            scope: Some(vec![]),
            token_type: Some("bearer".into()),
        };

        let refreshed = refreshed_twitch_token_state(&previous, exchange, 20).unwrap();
        assert_eq!(
            refreshed,
            TwitchTokenState {
                access_token: "new-access".into(),
                refresh_token: "new-refresh".into(),
                expires_in_seconds: None,
                scope: vec![],
                token_type: "bearer".into(),
                linked_at_ms: 20,
            }
        );
    }

    #[test]
    fn signed_auth_token_round_trips() {
        let claims = SignedAuthClaims {
            identity: AfkIdentity::new("1", "tester", "Tester"),
            issued_at_ms: 10,
            expires_at_ms: 20,
        };
        let token = sign_auth_token(SECRET, &claims).expect("token should sign");
        let decoded = verify_auth_token(SECRET, &token, 15).expect("token should verify");
        assert_eq!(decoded, claims);
    }

    #[test]
    fn callback_redirect_url_restores_query_route() {
        let redirect = callback_redirect_url("http://localhost:4365/detonito", Some("/?view=afk"))
            .expect("redirect should build");
        assert_eq!(
            redirect.as_str(),
            "http://localhost:4365/detonito/?view=afk"
        );
    }
}
