#![allow(dead_code)]

// filename according to https://doc.rust-lang.org/book/ch11-03-test-organization.html
use embedded_storage::nor_flash::{
    ErrorType, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

pub const FLASH_SECTOR_SIZE: usize = 4096;
pub const PAGE_HEADER_SIZE: usize = 32;
pub const ENTRY_STATE_MAP_OFFSET: usize = PAGE_HEADER_SIZE;

pub const ENTRY_STATE_MAP_SIZE: usize = 32;
pub const ENTRY_STATE_MAP_ENTRY_SIZE: usize = 1;

pub const ITEM_OFFSET: usize = PAGE_HEADER_SIZE + ENTRY_STATE_MAP_SIZE;
pub const ITEM_SIZE: usize = 32;
// 1 byte is the minimum that can be written

#[derive(Default)]
pub struct Flash {
    pub buf: Vec<u8>,
    pub fail_after_operation: usize,
    pub operations: Vec<Operation>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Operation {
    Read { offset: u32, len: usize },
    Write { offset: u32, len: usize },
    Erase { offset: u32, len: usize },
}

impl Flash {
    pub fn new(pages: usize) -> Self {
        Self {
            buf: vec![0xffu8; FLASH_SECTOR_SIZE * pages],
            fail_after_operation: usize::MAX,
            ..Default::default()
        }
    }

    pub fn new_with_fault(pages: usize, fail_after_operation: usize) -> Self {
        Self {
            buf: vec![0xffu8; FLASH_SECTOR_SIZE * pages],
            fail_after_operation,
            ..Default::default()
        }
    }

    pub fn new_from_file(path: &str) -> Self {
        let partition = std::fs::read(path).unwrap();
        Self {
            buf: partition,
            fail_after_operation: usize::MAX,
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn disable_faults(&mut self) {
        self.fail_after_operation = usize::MAX;
    }

    pub fn erases(&mut self) -> usize {
        self.operations
            .iter()
            .filter(|op| match op {
                Operation::Erase { .. } => true,
                _ => false,
            })
            .count()
    }

    pub fn dump_operations(&self) {
        println!("Operations:");
        for op in &self.operations {
            println!("  {:?}", op);
        }
    }
}

#[derive(Debug)]
pub struct FlashError;

impl NorFlashError for FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

impl ErrorType for Flash {
    type Error = FlashError;
}

impl ReadNorFlash for Flash {
    const READ_SIZE: usize = FLASH_SECTOR_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        println!(
            "    flash: read:  0x{offset:04X}[0x{:04X}] #{:>2}",
            bytes.len(),
            self.operations.len()
        );
        if self.operations.len() >= self.fail_after_operation {
            println!("    flash: FAULT");
            return Err(FlashError);
        }
        self.operations.push(Operation::Read {
            offset,
            len: bytes.len(),
        });

        let offset = offset as usize;
        bytes.copy_from_slice(&self.buf[offset..offset + bytes.len()]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.buf.len()
    }
}

impl NorFlash for Flash {
    const WRITE_SIZE: usize = FLASH_SECTOR_SIZE;

    const ERASE_SIZE: usize = FLASH_SECTOR_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        println!(
            "    flash: erase: {from:04X} - {to:04X} #{:>2}",
            self.operations.len()
        );

        if self.operations.len() >= self.fail_after_operation {
            println!("    flash: FAULT");
            return Err(FlashError);
        }

        assert!((to - from).is_multiple_of(FLASH_SECTOR_SIZE as u32));
        assert!(from.is_multiple_of(FLASH_SECTOR_SIZE as u32));

        self.operations.push(Operation::Erase {
            offset: from,
            len: (to - from) as usize,
        });

        for addr in from..to {
            self.buf[addr as usize] = 0xff;
        }
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        println!(
            "    flash: write: 0x{offset:04X}[0x{:04X}] #{:>2}",
            bytes.len(),
            self.operations.len()
        );

        if self.operations.len() >= self.fail_after_operation {
            println!("    flash: FAULT");
            return Err(FlashError);
        }
        assert!(bytes.len() > 0);

        self.operations.push(Operation::Write {
            offset,
            len: bytes.len(),
        });

        let offset = offset as usize;
        for (i, &val) in bytes.iter().enumerate() {
            // the esp flash we can only flip bits from 1 to 0
            // println!("0x[{:04x}] {} &= {val} = {}",  offset+i,self.buf[offset + i], self.buf[offset + i] & val);
            self.buf[offset + i] &= val;
            assert_eq!(self.buf[offset + i], val, "detected improper flash write");
        }
        Ok(())
    }
}

impl esp_nvs::platform::Crc for Flash {
    fn crc32(init: u32, data: &[u8]) -> u32 {
        unsafe { libz_sys::crc32(init as u64, data.as_ptr(), data.len() as u32) as u32 }
    }
}
