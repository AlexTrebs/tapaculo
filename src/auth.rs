//! JWT authentication module for signing and verifying access and refresh tokens.
//!
//! This module provides a simple interface for generating and validating JWTs
//! for multi-user game sessions. Each access token includes a user ID (`sub`), room ID (`room`),
//! and expiration timestamp (`exp`). Refresh tokens allow clients to obtain new access tokens
//! without re-authenticating, if still valid.
//!
//! ## Example Usage
//! ```
//! use tapaculo::auth::JwtAuth;
//!
//! let auth = JwtAuth::new("super-secret-key");
//! let access = auth.sign_access("user42".into(), "room99".into(), 3600).unwrap();
//! let refresh = auth.sign_refresh("user42".into(), 86400).unwrap();
//! let claims = auth.verify_access(&access).unwrap();
//! assert_eq!(claims.sub, "user42");
//! assert_eq!(claims.room, "room99");
//! ```
use jsonwebtoken::{encode, decode, Header, Validation, EncodingKey, DecodingKey};
use serde::{Serialize, Deserialize};
use anyhow::{Context, Result};

/// Represents the payload of an access token used for multi-user session authentication.
///
/// ## Fields
/// - `sub`: Subject — the unique identifier of the user (e.g. user ID).
/// - `room`: Custom claim — the ID of the room or session the user is joining.
/// - `exp`: Expiration — UNIX timestamp when the token should expire.
/// - `iss`: Issuer — optional identifier of the token issuer (e.g. your server name).
/// - `aud`: Audience — optional identifier of the intended recipient (e.g. client app).
#[derive(Debug, Serialize, Deserialize)]
pub struct AccessClaims {
  pub sub: String,
  pub room: String,
  pub exp: usize,
  pub iss: Option<String>,
  pub aud: Option<String>,
}

/// Represents the payload of a refresh token used to obtain new access tokens.
///
/// ## Fields
/// - `sub`: Subject — the user ID.
/// - `exp`: Expiration — UNIX timestamp when the refresh token expires.
#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshClaims {
  pub sub: String,
  pub exp: usize,
}

/// Configuration options for JWT validation.
///
/// ## Fields
/// - `leeway`: Allowed clock skew in seconds.
/// - `issuer`: Optional expected issuer string.
/// - `audience`: Optional expected audience string.
#[derive(Clone)]
pub struct JwtAuthOptions {
  pub leeway: u64,
  pub issuer: Option<String>,
  pub audience: Option<String>,
}

impl Default for JwtAuthOptions {
  fn default() -> Self {
    Self {
      leeway: 0,
      issuer: None,
      audience: None,
    }
  }
}

/// JWT authentication handler for signing and verifying tokens.
#[derive(Clone)]
pub struct JwtAuth {
  secret: String,
  options: JwtAuthOptions,
}

impl JwtAuth {
  /// Creates a new instance of `JwtAuth`.
  ///
  /// ## Parameters
  /// - `secret`: The signing key used to encode and decode JWTs.
  ///
  /// ## Example
  /// ```
  /// use tapaculo::auth::{JwtAuth, JwtAuthOptions};
  /// let auth = JwtAuth::new("my-secret-key");
  /// ```
  pub fn new(secret: &str) -> Self {
    Self {
      secret: secret.into(),
      options: JwtAuthOptions::default()
    }
  }

  /// Creates a new instance of `JwtAuth` with configurable options.
  ///
  /// ## Parameters
  /// - `secret`: The signing key used to encode and decode JWTs.
  /// - `options`: Configuration options including leeway, issuer, and audience.
  ///
  /// ## Example
  /// ```
  /// use tapaculo::auth::{JwtAuth, JwtAuthOptions};
  /// let auth = JwtAuth::with_options("my-secret-key", JwtAuthOptions::default());
  /// ```
  pub fn with_options(secret: &str, options: JwtAuthOptions) -> Self {
    Self {
      secret: secret.into(),
      options,
    }
  }

  /// Signs an access token for a user and room with a custom expiry.
  ///
  /// ## Parameters
  /// - `user_id`: ID of the user seeking authentication.
  /// - `room_id`: ID of the room the user is accessing.
  /// - `ttl_secs`: Time-to-live in seconds for the token.
  ///
  /// ## Returns
  /// - `Result<String>`: Encoded JWT token or an error.
  pub fn sign_access(&self, user_id: String, room_id: String, ttl_secs: usize) -> Result<String> {
    let now = chrono::Utc::now().timestamp();
    let exp = now.saturating_add(ttl_secs as i64) as usize;
    
    let claims = AccessClaims {
      sub: user_id,
      room: room_id,
      exp,
      iss: self.options.issuer.clone(),
      aud: self.options.audience.clone(),
    };
    encode(
      &Header::default(), 
      &claims, 
      &EncodingKey::from_secret(self.secret.as_ref())
    )
    .context("Failed to encode access token.")
  }

  /// Signs a refresh token for a user with a longer expiry.
  ///
  /// ## Parameters
  /// - `user_id`: ID of the user.
  /// - `ttl_secs`: Time-to-live in seconds for the refresh token.
  ///
  /// ## Returns
  /// - `Result<String>`: Encoded refresh token or an error.
  pub fn sign_refresh(&self, user_id: String, ttl_secs: usize) -> Result<String> {
    let exp = chrono::Utc::now().timestamp() as usize + ttl_secs;
    let claims = RefreshClaims {
      sub: user_id,
      exp,
    };
    encode(
      &Header::default(), 
      &claims, 
      &EncodingKey::from_secret(self.secret.as_ref())
    )
    .context("Failed to encode refresh token.")
  }

  /// Verifies an access token and returns its claims if valid.
  ///
  /// ## Parameters
  /// - `token`: Encoded JWT access token.
  ///
  /// ## Returns
  /// - `Result<AccessClaims>`: Parsed claims if valid.
  pub fn verify_access(&self, token: &str) -> Result<AccessClaims> {
    let mut validation = Validation::default();
    validation.leeway = self.options.leeway;
    if let Some(ref iss) = self.options.issuer {
      validation.set_issuer(&[iss]);
    }
    if let Some(ref aud) = self.options.audience {
      validation.set_audience(&[aud]);
    }
    let data = decode::<AccessClaims>(
      token,
      &DecodingKey::from_secret(self.secret.as_ref()),
      &validation,
    ).context("Failed to decode access token")?;
    Ok(data.claims)
  }

  /// Verifies a refresh token and returns its claims if valid.
  ///
  /// ## Parameters
  /// - `token`: Encoded JWT refresh token.
  ///
  /// ## Returns
  /// - `Result<RefreshClaims>`: Parsed claims if valid.
  pub fn verify_refresh(&self, token: &str) -> Result<RefreshClaims> {
    let mut validation = Validation::default();
    validation.leeway = self.options.leeway;
    let data = decode::<RefreshClaims>(
      token,
      &DecodingKey::from_secret(self.secret.as_ref()),
      &validation,
    ).context("Failed to decode refresh token")?;
    Ok(data.claims)
  }

  /// Refreshes an access token using a valid refresh token.
  ///
  /// ## Parameters
  /// - `refresh_token`: The refresh token string.
  /// - `room_id`: The room to re-issue access for.
  /// - `access_ttl`: Time-to-live in seconds for the new access token.
  ///
  /// ## Returns
  /// - `Result<String>`: New access token.
  pub fn refresh_access(&self, refresh_token: &str, room_id: String, access_ttl: usize) -> Result<String> {
    let claims = self.verify_refresh(refresh_token)?;
    self.sign_access(claims.sub, room_id, access_ttl)
  }
}


/// ######################################## TESTS ########################################

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;
  use std::thread::sleep;

  fn auth() -> JwtAuth {
    JwtAuth::new("test-secret")
  }

  #[test]
  fn access_token_roundtrip() {
    let auth = auth();
    let token = auth.sign_access("user1".into(), "roomA".into(), 60).unwrap();
    let claims = auth.verify_access(&token).unwrap();
    assert_eq!(claims.sub, "user1");
    assert_eq!(claims.room, "roomA");
  }

  #[test]
  fn refresh_token_roundtrip() {
    let auth = auth();
    let token = auth.sign_refresh("user2".into(), 60).unwrap();
    let claims = auth.verify_refresh(&token).unwrap();
    assert_eq!(claims.sub, "user2");
  }

  #[test]
  fn refresh_access_flow() {
    let auth = auth();
    let refresh = auth.sign_refresh("user3".into(), 60).unwrap();
    let new_access = auth.refresh_access(&refresh, "roomB".into(), 60).unwrap();
    let claims = auth.verify_access(&new_access).unwrap();
    assert_eq!(claims.sub, "user3");
    assert_eq!(claims.room, "roomB");
  }

  #[test]
  fn expired_access_token_fails() {
    let auth = auth();
    let token = auth.sign_access("user4".into(), "roomC".into(), 1).unwrap();
    sleep(Duration::from_secs(2));
    assert!(auth.verify_access(&token).is_err());
  }

  #[test]
  fn expired_refresh_token_fails() {
    let auth = auth();
    let token = auth.sign_refresh("user5".into(), 1).unwrap();
    sleep(Duration::from_secs(2));
    assert!(auth.verify_refresh(&token).is_err());
  }
}
