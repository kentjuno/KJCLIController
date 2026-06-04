use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

/// Constant-time comparison to prevent timing attacks.
pub fn secure_compare(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    if a_bytes.len() != b_bytes.len() {
        return false;
    }
    let mut result = 0;
    for (x, y) in a_bytes.iter().zip(b_bytes.iter()) {
        result |= x ^ y;
    }
    result == 0
}

pub async fn auth_middleware(
    request: Request<Body>,
    next: Next,
    expected_token: String,
) -> Result<Response, StatusCode> {
    // Read the authorization header
    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = &auth_str[7..];
                if secure_compare(token, &expected_token) {
                    return Ok(next.run(request).await);
                }
            }
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}
