/// Fixes a garbled 32-bit version number reported by some older mIRC versions.
pub(crate) fn fix_m_version(m_version: u32) -> u32 {
    let mut major = m_version & 0xFFFF; // Low word
    let mut minor = (m_version >> 16) & 0xFFFF; // High word

    // 6.1, 6.2
    // Corrects Bug #2 for v6.1 (6.01 -> 6.10) and v6.2 (6.02 -> 6.20)
    if major == 6 && minor <= 2 {
        minor *= 10;
    }

    // Corrects versions that incorrectly reported major = 0 (Bug #1)
    if major == 0 {
        // 6.00 - 6.03
        // These versions had Bug #1 but not Bug #2
        if minor <= 3 {
            major = 6;
        } else {
            // 5.xx
            // All v5.x versions had Bug #1
            major = 5;
            if minor < 10 && minor >= 8 {
                // 5.8, 5.9
                // These two versions had both Bug #1 and Bug #2
                // (0.08 -> 5.80, 0.09 -> 5.90)
                minor *= 10;
            }
            // Versions like 5.81 (0.81), 5.82 (0.82), 5.91 (0.91)
            // are corrected by `major = 5` and fall through here,
            // as their minor versions were already correct.
        }
    }

    // Repack the corrected version
    (minor << 16) | major
}
