//! SHA-256 of `Deploy.exe` in each official SMF release this build knows.
//! Taken from the release's own `Release.zip` on GitHub. A later stage reads
//! upstream's releases at run time, so this table only has to cover releases
//! seen while building.

const KNOWN: &[(&str, &str)] = &[("f24f472f0d071c90f293bb67e49b9834d5c01188b3c2d278a3e04af86cd6907b", "2.33.42")];

pub fn version_for(sha256: &str) -> Option<&'static str> {
    KNOWN.iter().find(|(hash, _)| hash.eq_ignore_ascii_case(sha256)).map(|(_, v)| *v)
}
