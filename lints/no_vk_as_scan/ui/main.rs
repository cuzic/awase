// UI test: verify NO_VK_AS_SCAN fires on the cross-namespace conversion.
// The `good` call must NOT trigger a warning.

/// Minimal inline types matching the real awase::types definitions.
mod types {
    #[derive(Clone, Copy)]
    pub struct VkCode(u16);
    impl VkCode {
        pub const fn new(v: u16) -> Self { Self(v) }
        pub const fn as_u16(self) -> u16 { self.0 }
    }
    impl From<u16> for VkCode { fn from(v: u16) -> Self { Self(v) } }
    impl From<VkCode> for u16 { fn from(v: VkCode) -> Self { v.0 } }

    #[derive(Clone, Copy)]
    pub struct ScanCode(u32);
    impl ScanCode {
        pub const fn new(v: u32) -> Self { Self(v) }
    }
    impl From<u32> for ScanCode { fn from(v: u32) -> Self { Self(v) } }
}

use types::{ScanCode, VkCode};

fn main() {
    let vk = VkCode::new(0x1C);

    // Should trigger NO_VK_AS_SCAN: extracting VK numeric value and reusing as scan code
    let _ = ScanCode::new(u32::from(vk.as_u16())); //~ WARN constructing `ScanCode` from a `VkCode` value

    // Should NOT trigger: using a plain u32 literal
    let _ = ScanCode::new(0x1E_u32);
}
