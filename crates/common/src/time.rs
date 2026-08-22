//! `time::OffsetDateTime` ⇄ `prost_types::Timestamp`.

use prost_types::Timestamp;
use time::OffsetDateTime;

pub fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

pub fn to_proto(t: OffsetDateTime) -> Timestamp {
    Timestamp {
        seconds: t.unix_timestamp(),
        nanos: t.nanosecond().cast_signed(),
    }
}

pub fn to_proto_opt(t: Option<OffsetDateTime>) -> Option<Timestamp> {
    t.map(to_proto)
}

pub fn from_proto(t: &Timestamp) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(t.seconds)
        .ok()
        .and_then(|d| d.replace_nanosecond(t.nanos.cast_unsigned()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = now();
        let p = to_proto(t);
        assert_eq!(from_proto(&p).unwrap(), t);
    }
}
