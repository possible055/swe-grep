#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let cleaned = s.trim().trim_start_matches(['v', 'V']);
        let mut parts = cleaned.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch_part = parts.next()?;
        let patch = patch_part
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()?;
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

pub fn is_newer(target: &str, current: &str) -> bool {
    match (Version::parse(target), Version::parse(current)) {
        (Some(t), Some(c)) => t > c,
        _ => target.trim() != current.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(
            Version::parse("0.1.8"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 8
            })
        );
        assert_eq!(
            Version::parse("v0.1.9"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 9
            })
        );
        assert_eq!(
            Version::parse("v1.2.3-beta.1"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.1.9", "0.1.8"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.8", "0.1.8"));
        assert!(!is_newer("v0.1.7", "0.1.8"));
    }
}
