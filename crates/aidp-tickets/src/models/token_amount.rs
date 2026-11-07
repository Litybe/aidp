use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const THOUSAND: f64 = 1_000.0;
const MILLION: f64 = 1_000_000.0;
const BILLION: f64 = 1_000_000_000.0;

/// A count of tokens written as a number (`3000000`) or with a suffix (`3M`, `1.5M`, `500k`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TokenAmount(pub u64);

impl TokenAmount {
    /// Parses `3M`, `1.5m`, `500k`, `2G` or a plain number; blanks and `_` are ignored.
    pub fn parse(text: &str) -> Option<TokenAmount> {
        let cleaned: String = text
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '_')
            .collect::<String>()
            .to_ascii_lowercase();
        let (digits, factor) = match cleaned.chars().last()? {
            'k' => (&cleaned[..cleaned.len() - 1], THOUSAND),
            'm' => (&cleaned[..cleaned.len() - 1], MILLION),
            'g' => (&cleaned[..cleaned.len() - 1], BILLION),
            _ => (cleaned.as_str(), 1.0),
        };
        let value: f64 = digits.replace(',', ".").parse().ok()?;
        (value.is_finite() && value >= 0.0).then(|| TokenAmount((value * factor).round() as u64))
    }
}

impl Serialize for TokenAmount {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

struct AmountVisitor;

impl Visitor<'_> for AmountVisitor {
    type Value = TokenAmount;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a token count such as 3000000, 3M or 500k")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<TokenAmount, E> {
        Ok(TokenAmount(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<TokenAmount, E> {
        u64::try_from(value)
            .map(TokenAmount)
            .map_err(|_| E::custom("a token count cannot be negative"))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<TokenAmount, E> {
        if value.is_finite() && value >= 0.0 {
            Ok(TokenAmount(value.round() as u64))
        } else {
            Err(E::custom("a token count must be a positive number"))
        }
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<TokenAmount, E> {
        TokenAmount::parse(value).ok_or_else(|| E::custom(format!("invalid token count `{value}`")))
    }
}

impl<'de> Deserialize<'de> for TokenAmount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(AmountVisitor)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/token_amount_tests.rs"]
mod tests;
