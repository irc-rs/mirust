use core::fmt;

/// An mIRC version number, such as `7.85`.
///
/// The minor part is stored as mIRC writes it after the dot, padded to two digits:
/// `6.2` is `Version::new(6, 20)`, `7.01` is `Version::new(7, 1)`. Versions compare in
/// release order.
///
/// # Reporting bugs in old mIRC versions
///
/// [`Version::from_raw`] corrects two bugs in the `mVersion` value that mIRC 5.8 – 6.2
/// passed to `LoadDll`. Both were confirmed by disassembling the affected releases:
///
/// 1. **Major reported as 0 (5.8 – 6.03).** mIRC built its version string as `"v6.03"` and
///    ran `atoi` over it; the leading `v` made the major part parse as `0`.
/// 2. **Minor not padded (5.8 – 6.2).** The minor part was parsed as written, so `6.2`
///    reported a minor of `2` rather than `20`, and `5.8` reported `8`. mIRC 6.21 started
///    right-padding the minor with zeros (changelog: *"The LOADINFO structure now returns a
///    minor version value that is right-padded with zeros"*).
///
/// Together they give these raw values (major, minor):
///
/// | Release        | Reported      | Bugs   |
/// |----------------|---------------|--------|
/// | 5.8, 5.9       | `0.8`, `0.9`  | 1 and 2 |
/// | 5.81, 5.82, 5.91 | `0.81`, …   | 1       |
/// | 6.0 – 6.03     | `0.0` – `0.3` | 1       |
/// | 6.1, 6.2       | `6.1`, `6.2`  | 2       |
/// | 6.11 – 6.17, 6.21+ | correct   | –       |
///
/// None of the faulty values collide with a genuine release, so the correction is exact.
/// Verified in real mIRC for 6.03 (reports `0.3`) and 6.2 (reports `6.2`), and for correct
/// reports from 6.12, 6.14 – 6.17, 6.21, 6.3 and later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u16,
    minor: u16,
}

impl Version {
    /// mIRC 5.6, the first release with `/dll` support. Did not call `LoadDll`.
    pub const V5_6: Self = Self::new(5, 60);
    /// mIRC 5.8, which added `LoadDll`, `UnloadDll`, `mVersion`, `mHwnd` and `mKeep`.
    pub const V5_8: Self = Self::new(5, 80);
    /// mIRC 6.1, which added `$dllcall()`.
    pub const V6_1: Self = Self::new(6, 10);
    /// mIRC 6.32, which raised the `data` / `parms` buffers from ~900 to 4200 bytes.
    pub const V6_32: Self = Self::new(6, 32);
    /// mIRC 7.0, which added `mUnicode`.
    pub const V7_0: Self = Self::new(7, 0);
    /// mIRC 7.51, which added `mBeta`.
    pub const V7_51: Self = Self::new(7, 51);
    /// mIRC 7.64, which added `mBytes`.
    pub const V7_64: Self = Self::new(7, 64);
    /// mIRC 7.84, which fixed `mBytes` being reported as a UTF-16 byte count.
    pub const V7_84: Self = Self::new(7, 84);

    /// Creates a version from its major and (two-digit) minor parts.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Decodes an `mVersion` value from `LOADINFO`, correcting the bugs described in the
    /// [type-level docs](Version#reporting-bugs-in-old-mirc-versions).
    pub const fn from_raw(raw: u32) -> Self {
        let major = (raw & 0xFFFF) as u16;
        let minor = (raw >> 16) as u16;

        match (major, minor) {
            // "v6.0" .. "v6.03": major lost, minor already two digits.
            (0, 0..=3) => Self::new(6, minor),
            // "v5.8", "v5.9": major lost and minor unpadded.
            (0, 8 | 9) => Self::new(5, minor * 10),
            // "v5.81", "v5.82", "v5.91": major lost.
            (0, _) => Self::new(5, minor),
            // "6.1", "6.2": minor unpadded. (6.01 / 6.02 report major 0, so no clash.)
            (6, 1 | 2) => Self::new(6, minor * 10),
            _ => Self::new(major, minor),
        }
    }

    /// Encodes the version the way current mIRC does: major in the low word, minor in the
    /// high word.
    pub const fn to_raw(self) -> u32 {
        (self.minor as u32) << 16 | self.major as u32
    }

    /// The major part, e.g. `7` for 7.85.
    pub const fn major(self) -> u16 {
        self.major
    }

    /// The two-digit minor part, e.g. `85` for 7.85 and `20` for 6.2.
    pub const fn minor(self) -> u16 {
        self.minor
    }
}

impl fmt::Display for Version {
    /// Formats as mIRC does: `7.85`, `7.01`, `6.2`, `6.0`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.minor % 10 == 0 && self.minor < 100 {
            write!(f, "{}.{}", self.major, self.minor / 10)
        } else {
            write!(f, "{}.{:02}", self.major, self.minor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(major: u32, minor: u32) -> u32 {
        minor << 16 | major
    }

    #[test]
    fn corrects_every_buggy_release() {
        let cases = [
            // (reported major, reported minor) -> actual
            ((0, 8), (5, 80)),
            ((0, 81), (5, 81)),
            ((0, 82), (5, 82)),
            ((0, 9), (5, 90)),
            ((0, 91), (5, 91)),
            ((0, 0), (6, 0)),
            ((0, 1), (6, 1)),
            ((0, 2), (6, 2)),
            ((0, 3), (6, 3)),
            ((6, 1), (6, 10)),
            ((6, 2), (6, 20)),
        ];
        for ((maj, min), (want_maj, want_min)) in cases {
            assert_eq!(
                Version::from_raw(raw(maj, min)),
                Version::new(want_maj, want_min),
                "raw {maj}.{min}"
            );
        }
    }

    #[test]
    fn leaves_correct_releases_alone() {
        for (maj, min) in [
            (6, 11),
            (6, 17),
            (6, 21),
            (6, 35),
            (7, 0),
            (7, 1),
            (7, 64),
            (7, 85),
        ] {
            assert_eq!(
                Version::from_raw(raw(maj, min)),
                Version::new(maj as u16, min as u16)
            );
        }
    }

    #[test]
    fn corrected_versions_sort_in_release_order() {
        let releases = [
            (0, 8),
            (0, 81),
            (0, 82),
            (0, 9),
            (0, 91),
            (0, 0),
            (0, 1),
            (0, 2),
            (0, 3),
        ];
        let releases =
            releases
                .into_iter()
                .chain([(6, 1), (6, 11), (6, 17), (6, 2), (6, 21), (7, 0)]);
        let versions: Vec<_> = releases
            .map(|(a, b)| Version::from_raw(raw(a, b)))
            .collect();
        assert!(versions.is_sorted(), "{versions:?}");
    }

    #[test]
    fn round_trips_raw() {
        assert_eq!(
            Version::from_raw(Version::new(7, 85).to_raw()),
            Version::new(7, 85)
        );
    }

    #[test]
    fn displays_like_mirc() {
        assert_eq!(Version::new(7, 85).to_string(), "7.85");
        assert_eq!(Version::new(7, 1).to_string(), "7.01");
        assert_eq!(Version::new(6, 20).to_string(), "6.2");
        assert_eq!(Version::new(6, 0).to_string(), "6.0");
        assert_eq!(Version::new(5, 91).to_string(), "5.91");
    }
}
