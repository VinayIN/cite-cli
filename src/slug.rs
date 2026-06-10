use serde::de::{self, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Slug(pub(crate) String);

impl Slug {
    pub fn new(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Err("Slug cannot be empty".into());
        }
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(format!(
                "Slug '{}' contains invalid characters (only a-z, 0-9, hyphens allowed)",
                s
            ));
        }
        if s.starts_with('-') || s.ends_with('-') {
            return Err(format!("Slug '{}' cannot start or end with a hyphen", s));
        }
        if s.contains("--") {
            return Err(format!("Slug '{}' cannot contain consecutive hyphens", s));
        }
        if s.to_lowercase() != s {
            return Err(format!("Slug '{}' must be lowercase", s));
        }
        Ok(Slug(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Slug {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_string(SlugVisitor)
    }
}

struct SlugVisitor;

impl<'de> Visitor<'de> for SlugVisitor {
    type Value = Slug;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a kebab-case string (a-z, 0-9, hyphens)")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Slug, E> {
        Slug::new(v).map_err(de::Error::custom)
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Slug {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_slugs() {
        assert!(Slug::new("hello-world").is_ok());
        assert!(Slug::new("my-article-1").is_ok());
        assert!(Slug::new("a").is_ok());
        assert!(Slug::new("123").is_ok());
        assert!(Slug::new("test-123").is_ok());
    }

    #[test]
    fn invalid_slugs() {
        assert!(Slug::new("").is_err());
        assert!(Slug::new("Hello-World").is_err());
        assert!(Slug::new("-leading").is_err());
        assert!(Slug::new("trailing-").is_err());
        assert!(Slug::new("double--hyphen").is_err());
        assert!(Slug::new("space space").is_err());
        assert!(Slug::new("UPPERCASE").is_err());
        assert!(Slug::new("special!").is_err());
    }

    #[test]
    fn deserialize_valid() {
        let s: Slug = serde_yaml::from_str("hello-world").unwrap();
        assert_eq!(s.as_str(), "hello-world");
    }

    #[test]
    fn deserialize_invalid() {
        let result: Result<Slug, _> = serde_yaml::from_str("UPPERCASE");
        assert!(result.is_err());
    }
}
