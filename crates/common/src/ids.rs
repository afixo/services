//! Identifier helpers. All ids are UUID v7 (time-ordered: friendlier to btree
//! indexes than v4, and sortable in logs).

use tonic::Status;
use uuid::Uuid;

pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Parse a caller-supplied id, answering INVALID_ARGUMENT with the field name.
pub fn parse(field: &'static str, raw: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(raw).map_err(|_| Status::invalid_argument(format!("{field}: not a uuid")))
}

/// Same, for optional fields: empty string or None → None.
pub fn parse_opt(field: &'static str, raw: Option<&str>) -> Result<Option<Uuid>, Status> {
    match raw {
        None | Some("") => Ok(None),
        Some(s) => parse(field, s).map(Some),
    }
}

/// Handles are the public reference in `/v1/disclose/:handle`: 2–39 chars,
/// `[a-z0-9-]`, no leading/trailing hyphen (a superset of GitHub's login rules,
/// lower-cased).
pub fn validate_handle(handle: &str) -> Result<(), Status> {
    let ok = (2..=39).contains(&handle.len())
        && handle
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !handle.starts_with('-')
        && !handle.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "handle: must match [a-z0-9-]{2,39}",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles() {
        assert!(validate_handle("alice").is_ok());
        assert!(validate_handle("a-1").is_ok());
        assert!(validate_handle("Alice").is_err());
        assert!(validate_handle("-a").is_err());
        assert!(validate_handle("a").is_err());
    }

    #[test]
    fn ids_are_v7_and_parse() {
        let id = new_id();
        assert_eq!(id.get_version_num(), 7);
        assert_eq!(parse("x", &id.to_string()).unwrap(), id);
        assert!(parse("x", "nope").is_err());
        assert_eq!(parse_opt("x", Some("")).unwrap(), None);
    }
}
