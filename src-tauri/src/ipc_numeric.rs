use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Largest integer JavaScript can represent without precision loss.
pub const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;

/// A nullable integer that crosses IPC without changing its JSON or Specta
/// shape. Command return values cannot attach a serde field adapter directly,
/// so this transparent wrapper keeps `number | null` while enforcing the same
/// JavaScript-safe range as model fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
pub struct IpcOptionalU64(
    #[serde(with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub Option<u64>,
);

impl From<Option<u64>> for IpcOptionalU64 {
    fn from(value: Option<u64>) -> Self {
        Self(value)
    }
}

fn ensure_safe(value: u64) -> Result<u64, String> {
    if value <= MAX_SAFE_INTEGER {
        Ok(value)
    } else {
        Err(format!(
            "integer {value} exceeds JavaScript Number.MAX_SAFE_INTEGER ({MAX_SAFE_INTEGER})"
        ))
    }
}

pub mod u64 {
    use super::*;

    pub fn serialize<S>(value: &std::primitive::u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ensure_safe(*value)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<std::primitive::u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = std::primitive::u64::deserialize(deserializer)?;
        ensure_safe(value).map_err(serde::de::Error::custom)
    }
}

pub mod option_u64 {
    use super::*;

    pub fn serialize<S>(
        value: &Option<std::primitive::u64>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(value) => {
                serializer.serialize_some(&ensure_safe(*value).map_err(serde::ser::Error::custom)?)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<std::primitive::u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<std::primitive::u64>::deserialize(deserializer)?
            .map(ensure_safe)
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct RequiredValue {
        #[serde(with = "crate::ipc_numeric::u64")]
        value: u64,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct OptionalValue {
        #[serde(with = "crate::ipc_numeric::option_u64")]
        value: Option<u64>,
    }

    #[test]
    fn safe_integer_boundary_round_trips_exactly() {
        let json = serde_json::to_string(&RequiredValue {
            value: MAX_SAFE_INTEGER,
        })
        .unwrap();
        assert_eq!(json, format!(r#"{{"value":{MAX_SAFE_INTEGER}}}"#));
        assert_eq!(
            serde_json::from_str::<RequiredValue>(&json).unwrap().value,
            MAX_SAFE_INTEGER
        );
    }

    #[test]
    fn values_above_safe_integer_boundary_fail_in_both_directions() {
        let unsafe_value = MAX_SAFE_INTEGER + 1;
        assert!(serde_json::to_string(&RequiredValue {
            value: unsafe_value
        })
        .unwrap_err()
        .to_string()
        .contains("Number.MAX_SAFE_INTEGER"));
        assert!(
            serde_json::from_str::<RequiredValue>(&format!(r#"{{"value":{unsafe_value}}}"#))
                .unwrap_err()
                .to_string()
                .contains("Number.MAX_SAFE_INTEGER")
        );
        assert!(serde_json::to_string(&OptionalValue {
            value: Some(unsafe_value)
        })
        .is_err());
    }

    #[test]
    fn transparent_command_value_keeps_the_wire_shape_and_safety_check() {
        assert_eq!(
            serde_json::to_string(&IpcOptionalU64(Some(MAX_SAFE_INTEGER))).unwrap(),
            MAX_SAFE_INTEGER.to_string()
        );
        assert_eq!(
            serde_json::to_string(&IpcOptionalU64(None)).unwrap(),
            "null"
        );
        assert!(serde_json::to_string(&IpcOptionalU64(Some(MAX_SAFE_INTEGER + 1))).is_err());
    }
}
