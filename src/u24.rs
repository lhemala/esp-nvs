use core::fmt::{Debug, Formatter};

#[derive(Copy, Clone, PartialEq, Ord, PartialOrd, Eq)]
#[allow(non_camel_case_types)]
#[repr(transparent)]
pub struct u24([u8; 3]);

impl Debug for u24 {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("0x{:0>6x}", self.to_u32()))
    }
}

impl u24 {
    pub fn to_u32(self) -> u32 {
        let u24([a, b, c]) = self;
        u32::from_le_bytes([a, b, c, 0])
    }

    pub fn from_u32(num: u32) -> Self {
        let [a, b, c, _] = num.to_le_bytes();
        u24([a, b, c])
    }
}
