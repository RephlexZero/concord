//! Snowflake IDs. Discord sends them as strings; people writing YAML may not
//! quote them. Accept both, always write strings.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Sf(pub u64);

impl fmt::Debug for Sf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for Sf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for Sf {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Sf {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Sf;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a Discord ID (snowflake), e.g. \"1000000000004232083\"")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Sf, E> {
                Ok(Sf(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Sf, E> {
                u64::try_from(v)
                    .map(Sf)
                    .map_err(|_| E::custom("a Discord ID can't be negative"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Sf, E> {
                v.trim()
                    .parse()
                    .map(Sf)
                    .map_err(|_| E::custom(format!("`{v}` is not a Discord ID (expected digits)")))
            }
        }
        d.deserialize_any(V)
    }
}
