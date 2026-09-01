use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Largest integer JavaScript can represent without precision loss.
pub const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;

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
}
