//! Small input-validation helpers (SEC-26 / SEC-27): trim, reject control
//! characters, and cap length on user-supplied text before it is persisted or
//! interpolated into outbound messages.

use crate::error::AppError;

/// Trim, reject control characters, and cap length (in chars). When `required`,
/// an empty (post-trim) value is rejected. Returns the cleaned value.
pub fn text(value: &str, label: &str, max: usize, required: bool) -> Result<String, AppError> {
    let v = value.trim();
    if required && v.is_empty() {
        return Err(AppError::BadRequest(format!("{label} is required")));
    }
    if v.chars().any(|c| c.is_control()) {
        return Err(AppError::BadRequest(format!(
            "{label} contains invalid control characters"
        )));
    }
    if v.chars().count() > max {
        return Err(AppError::BadRequest(format!(
            "{label} must be at most {max} characters"
        )));
    }
    Ok(v.to_string())
}

/// Validate an email: <= 320 chars, no whitespace/control characters, and a
/// `local@domain` shape with a dotted domain. Not RFC-complete, but rejects the
/// obvious junk that `contains('@')` let through (SEC-27).
pub fn email(value: &str) -> Result<String, AppError> {
    let v = value.trim();
    if v.is_empty() || v.len() > 320 || v.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    let (local, domain) = v
        .split_once('@')
        .ok_or_else(|| AppError::BadRequest("invalid email".into()))?;
    // Domain must be >=2 non-empty labels, none starting/ending with '-'
    // (rejects a@b..com, a@-.com, a@b. — RA-20).
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-'));
    if local.is_empty() || !domain_ok {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    Ok(v.to_string())
}

/// Strip control characters and cap length for values interpolated into
/// outbound messages (email subjects/bodies) — SEC-26.
pub fn sanitize_line(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_caps_and_rejects_controls() {
        assert!(text("ok", "name", 10, true).is_ok());
        assert!(text("", "name", 10, true).is_err());
        assert!(text(&"x".repeat(11), "name", 10, false).is_err());
        assert!(text("bad\u{0007}bell", "name", 50, false).is_err());
    }

    #[test]
    fn email_accepts_valid_rejects_junk() {
        assert!(email("a@b.com").is_ok());
        assert!(email("@").is_err());
        assert!(email("no-at-sign").is_err());
        assert!(email("a@b").is_err());
        assert!(email("a b@c.com").is_err());
        assert!(email("a@b..com").is_err()); // empty label (RA-20)
        assert!(email("a@-.com").is_err()); // hyphen-only label (RA-20)
        assert!(email("a@b.").is_err()); // trailing dot
        assert!(email(&format!("{}@b.com", "x".repeat(320))).is_err());
    }

    #[test]
    fn sanitize_strips_controls_and_caps() {
        assert_eq!(sanitize_line("a\r\nb", 100), "ab");
        assert_eq!(sanitize_line("abcdef", 3), "abc");
    }
}
