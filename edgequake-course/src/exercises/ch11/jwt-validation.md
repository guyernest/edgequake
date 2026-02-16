# Exercise: JWT Validation Middleware

::: exercise
id: ch11-jwt-validation
difficulty: intermediate
time: 30 minutes
:::

In chapter 11 you learned that EdgeQuake uses JWT-based authentication for
stateless request validation. Every API request carries a signed JWT in the
`Authorization: Bearer <token>` header. The middleware validates the signature,
checks expiration, and extracts tenant and role claims before the request
reaches any handler.

In this exercise you will implement JWT validation middleware for Axum that
extracts Bearer tokens, validates them with HMAC-SHA256, and propagates the
authenticated claims to downstream handlers.

::: objectives
thinking:
  - Understand JWT structure (header.payload.signature) and how HMAC signing works
  - Recognize why stateless authentication is important for horizontally scaled services
  - Consider the security implications of token validation (expiration, algorithm, claims)
doing:
  - Extract Bearer tokens from the Authorization header
  - Validate JWT signatures using the jsonwebtoken crate
  - Propagate claims through Axum request extensions
  - Handle error cases (missing token, invalid signature, expired token)
:::

::: discussion
- Why is JWT-based authentication preferred over session-based authentication for API servers?
- What is the difference between HS256 (HMAC) and RS256 (RSA) signing algorithms? When would you use each?
- How does the `tenant_id` claim enable multi-tenant isolation in EdgeQuake?
:::

::: starter file="src/main.rs"
```rust
use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// JWT claims for EdgeQuake API authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user ID).
    pub sub: String,
    /// Expiration time (UNIX timestamp).
    pub exp: usize,
    /// Issued at (UNIX timestamp).
    pub iat: usize,
    /// Tenant identifier for namespace isolation.
    pub tenant_id: String,
    /// User role (e.g., "admin", "reader", "writer").
    pub role: String,
}

/// Shared configuration for JWT validation.
#[derive(Clone)]
pub struct JwtConfig {
    pub secret: String,
}

/// JWT validation middleware for Axum.
///
/// This middleware:
/// 1. Extracts the Bearer token from the Authorization header
/// 2. Validates the JWT signature using the configured secret
/// 3. Checks that the token has not expired
/// 4. Inserts the validated Claims into request extensions
///
/// Returns 401 Unauthorized if any step fails.
pub async fn jwt_middleware(
    Extension(config): Extension<JwtConfig>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // TODO Step 1: Extract the Authorization header value.
    // - Get the "Authorization" header from request.headers()
    // - Convert it to a string (handle invalid UTF-8 gracefully)
    // - Return 401 if the header is missing
    let auth_header = todo!("Extract Authorization header");

    // TODO Step 2: Strip the "Bearer " prefix to get the raw token.
    // - Handle both "Bearer " and "bearer " (case-insensitive prefix)
    // - Return 401 if the prefix is missing
    let token = todo!("Strip Bearer prefix");

    // TODO Step 3: Validate the token signature and decode claims.
    // - Use jsonwebtoken::decode::<Claims>() with:
    //   - DecodingKey::from_secret(config.secret.as_bytes())
    //   - Validation::default() (validates exp automatically)
    // - Return 401 if decode fails (invalid signature, expired, etc.)
    let claims: Claims = todo!("Decode and validate JWT");

    // TODO Step 4: Insert validated claims into request extensions.
    // - Use request.extensions_mut().insert(claims)
    // - This allows downstream handlers to access claims via Extension<Claims>

    // TODO Step 5: Call the next middleware/handler and return the response.
    // - Use next.run(request).await
    todo!("Propagate request to next handler")
}

/// Example protected handler that uses the extracted claims.
async fn protected_handler(Extension(claims): Extension<Claims>) -> impl IntoResponse {
    Json(serde_json::json!({
        "message": "Access granted",
        "user": claims.sub,
        "tenant": claims.tenant_id,
        "role": claims.role,
    }))
}

/// Health check endpoint (no authentication required).
async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "healthy"}))
}

/// Helper: create a signed JWT for testing.
pub fn create_test_token(secret: &str, claims: &Claims) -> String {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("Failed to create test token")
}

/// Helper: get current UNIX timestamp.
pub fn now_timestamp() -> usize {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
}

#[tokio::main]
async fn main() {
    let config = JwtConfig {
        secret: "super-secret-key-for-development-only".to_string(),
    };

    let protected = Router::new()
        .route("/api/me", get(protected_handler))
        .layer(middleware::from_fn(jwt_middleware))
        .layer(Extension(config));

    let app = Router::new()
        .route("/health", get(health))
        .merge(protected);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}
```
:::

::: hint level=1 title="Extracting the Authorization header"
```rust
let auth_header = request
    .headers()
    .get(header::AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .ok_or(StatusCode::UNAUTHORIZED)?;
```
:::

::: hint level=2 title="Case-insensitive Bearer prefix stripping"
```rust
let token = if auth_header.len() > 7
    && auth_header[..7].eq_ignore_ascii_case("bearer ")
{
    &auth_header[7..]
} else {
    return Err(StatusCode::UNAUTHORIZED);
};
```
:::

::: hint level=3 title="Full decode and extension insertion"
```rust
let token_data = decode::<Claims>(
    token,
    &DecodingKey::from_secret(config.secret.as_bytes()),
    &Validation::default(),
)
.map_err(|_| StatusCode::UNAUTHORIZED)?;

let claims = token_data.claims;
request.extensions_mut().insert(claims);

Ok(next.run(request).await)
```
:::

::: solution
```rust
use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
    pub tenant_id: String,
    pub role: String,
}

#[derive(Clone)]
pub struct JwtConfig {
    pub secret: String,
}

pub async fn jwt_middleware(
    Extension(config): Extension<JwtConfig>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Step 1: Extract the Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Step 2: Strip the "Bearer " prefix (case-insensitive)
    let token = if auth_header.len() > 7
        && auth_header[..7].eq_ignore_ascii_case("bearer ")
    {
        &auth_header[7..]
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    // Step 3: Validate the token and decode claims
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(config.secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Step 4: Insert claims into request extensions
    let claims = token_data.claims;
    request.extensions_mut().insert(claims);

    // Step 5: Continue to the next handler
    Ok(next.run(request).await)
}

async fn protected_handler(Extension(claims): Extension<Claims>) -> impl IntoResponse {
    Json(serde_json::json!({
        "message": "Access granted",
        "user": claims.sub,
        "tenant": claims.tenant_id,
        "role": claims.role,
    }))
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "healthy"}))
}

pub fn create_test_token(secret: &str, claims: &Claims) -> String {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("Failed to create test token")
}

pub fn now_timestamp() -> usize {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
}

#[tokio::main]
async fn main() {
    let config = JwtConfig {
        secret: "super-secret-key-for-development-only".to_string(),
    };

    let protected = Router::new()
        .route("/api/me", get(protected_handler))
        .layer(middleware::from_fn(jwt_middleware))
        .layer(Extension(config));

    let app = Router::new()
        .route("/health", get(health))
        .merge(protected);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}
```

### Explanation

The JWT middleware follows a linear pipeline:

1. **Header extraction**: Reads `Authorization` from request headers. Missing headers
   immediately return 401.
2. **Prefix stripping**: Handles both `Bearer` and `bearer` for compatibility with
   different HTTP clients.
3. **Signature validation**: `jsonwebtoken::decode` verifies the HMAC-SHA256 signature
   against the shared secret and automatically checks `exp` (expiration).
4. **Claims propagation**: Inserting claims into `request.extensions_mut()` makes them
   available to any downstream handler via `Extension<Claims>`.
5. **Pass-through**: If validation succeeds, the request continues to the handler.
   If any step fails, a 401 response is returned without reaching the handler.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request as HttpRequest, StatusCode},
    };
    use tower::ServiceExt;

    const TEST_SECRET: &str = "test-secret-key-12345";

    fn make_claims(exp_offset_secs: i64) -> Claims {
        let now = now_timestamp();
        Claims {
            sub: "user-42".to_string(),
            exp: (now as i64 + exp_offset_secs) as usize,
            iat: now,
            tenant_id: "tenant-acme".to_string(),
            role: "admin".to_string(),
        }
    }

    fn build_app() -> Router {
        let config = JwtConfig {
            secret: TEST_SECRET.to_string(),
        };

        Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn(jwt_middleware))
            .layer(Extension(config))
    }

    #[tokio::test]
    async fn test_valid_token() {
        let app = build_app();
        let claims = make_claims(3600); // Expires in 1 hour
        let token = create_test_token(TEST_SECRET, &claims);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["user"], "user-42");
        assert_eq!(json["tenant"], "tenant-acme");
        assert_eq!(json["role"], "admin");
    }

    #[tokio::test]
    async fn test_missing_authorization_header() {
        let app = build_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_invalid_token_signature() {
        let app = build_app();

        // Sign with a different secret
        let claims = make_claims(3600);
        let token = create_test_token("wrong-secret", &claims);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_expired_token() {
        let app = build_app();

        // Token expired 1 hour ago
        let claims = make_claims(-3600);
        let token = create_test_token(TEST_SECRET, &claims);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_malformed_bearer_prefix() {
        let app = build_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("Authorization", "Token abc123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_lowercase_bearer_prefix() {
        let app = build_app();
        let claims = make_claims(3600);
        let token = create_test_token(TEST_SECRET, &claims);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("Authorization", format!("bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_completely_invalid_token() {
        let app = build_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/protected")
                    .header("Authorization", "Bearer not.a.jwt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
```
:::

::: reflection
- The middleware returns a bare 401 status code with no body. In production, would
  you include an error message? What are the security trade-offs?
- How would you extend this middleware to enforce role-based access control (e.g.,
  only `admin` can delete documents)?
- What happens if the JWT secret needs to be rotated? How would you support multiple
  valid secrets during a rotation window?
:::
