//! Flash driver for STM32Gx family
//!
//! Should work for any of the G0 or G4 family parts in single bank mode. No support for dual bank
//! currently

use embedded_storage::nor_flash::{ErrorType, NorFlashError, NorFlashErrorKind, ReadNorFlash};

use stm32_metapac as pac;

use core::{
    ptr::slice_from_raw_parts,
    sync::atomic::{Ordering, fence},
};

use crate::dual_page::{DualPageFlash, Page};

const PAGE_SIZE: usize = 2048;
const FLASH_BASE: usize = 0x0800_0000;

#[cfg(feature = "stm32g431xb")]
const NUM_PAGES: usize = 64;
#[cfg(feature = "stm32g431x8")]
const NUM_PAGES: usize = 32;
#[cfg(feature = "stm32g431x6")]
const NUM_PAGES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlashError {
    OutOfRange,
}

impl NorFlashError for FlashError {
    fn kind(&self) -> embedded_storage::nor_flash::NorFlashErrorKind {
        match self {
            Self::OutOfRange => NorFlashErrorKind::OutOfBounds,
        }
    }
}

pub struct Stm32gxFlash {}

impl ErrorType for Stm32gxFlash {
    type Error = FlashError;
}

impl Stm32gxFlash {
    pub fn new() -> Self {
        Self {}
    }

    pub fn in_range(addr: usize, size: usize) -> bool {
        if addr < FLASH_BASE {
            false
        } else if addr + size >= FLASH_BASE + NUM_PAGES * PAGE_SIZE {
            false
        } else {
            true
        }
    }
}

pub fn unlock_flash() {
    pac::FLASH.keyr().write_value(0x45670123);
    pac::FLASH.keyr().write_value(0xCDEF89AB);
}

pub fn lock_flash() {
    pac::FLASH.cr().modify(|w| w.set_lock(true));
}

pub fn wait_busy() {
    while pac::FLASH.sr().read().bsy() {}
}

pub fn erase_page(page_num: usize) {
    wait_busy();
    clear_errors();
    pac::FLASH.cr().modify(|w| {
        w.set_per(true);
        w.set_pnb(page_num as u8);
    });
    pac::FLASH.cr().modify(|w| {
        w.set_strt(true);
    });

    wait_busy();

    pac::FLASH.cr().modify(|w| w.set_per(false));
}

fn clear_errors() -> u32 {
    let sr = pac::FLASH.sr().read().0;
    // Clear error flags
    pac::FLASH.sr().modify(|w| {
        w.set_fasterr(true);
        w.set_miserr(true);
        w.set_operr(true);
        w.set_pgserr(true);
        w.set_pgaerr(true);
        w.set_progerr(true);
        w.set_rderr(true);
        w.set_sizerr(true);
        w.set_wrperr(true);
    });

    sr
}

impl ReadNorFlash for Stm32gxFlash {
    const READ_SIZE: usize = 1;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let addr = FLASH_BASE + offset as usize;
        if !Self::in_range(offset as usize, bytes.len()) {
            return Err(FlashError::OutOfRange);
        }

        let data = unsafe { core::slice::from_raw_parts(addr as *const u8, bytes.len()) };
        bytes.copy_from_slice(data);
        Ok(())
    }

    fn capacity(&self) -> usize {
        PAGE_SIZE * NUM_PAGES
    }
}

// impl NorFlash for Stm32gxFlash {
//     const WRITE_SIZE: usize = 64;

//     const ERASE_SIZE: usize = PAGE_SIZE;

//     fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
//         todo!()
//     }

//     fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
//         todo!()
//     }
// }

#[derive(Clone, Copy, Debug)]
struct Region {
    start_page: usize,
    size: usize,
}

impl Region {
    fn page_slice(&self) -> &'static [u8] {
        let addr = (FLASH_BASE + self.start_page * PAGE_SIZE) as *const u8;
        // Safety: I don't think the flash is going anywhere
        unsafe {
            slice_from_raw_parts(addr, PAGE_SIZE * self.size)
                .as_ref()
                .unwrap()
        }
    }

    fn page_ptr(&self, offset: usize) -> *mut u32 {
        (FLASH_BASE + self.start_page * PAGE_SIZE + offset) as *mut u32
    }
}

struct WriteRegion {
    region: Region,
    write_pos: usize,
    cache: [u8; 8],
}

impl WriteRegion {
    pub fn new(region: Region) -> Self {
        Self {
            region,
            write_pos: 0,
            cache: [0; 8],
        }
    }

    /// Erase all flash pages in the region
    pub fn erase(&mut self) {
        unlock_flash();
        let start = self.region.start_page;
        let end = start + self.region.size;
        for i in start..end {
            erase_page(i);
        }
        lock_flash();
    }

    pub fn write(&mut self, data: &[u8]) {
        let mut in_pos = 0;

        while in_pos < data.len() {
            let buf_pos = self.write_pos % 8;
            let to_copy = (8 - buf_pos).min(data.len() - in_pos);
            self.cache[buf_pos..buf_pos + to_copy].copy_from_slice(&data[in_pos..in_pos + to_copy]);
            in_pos += to_copy;
            self.write_pos += to_copy;
            if self.write_pos % 8 == 0 {
                self.write_cache(self.write_pos - 8);
            }
        }
    }

    /// Finish writing any cached write data
    ///
    /// Must be called after finishing all calls to write when write data is not aligned to the
    /// flash write word size of 64-bits
    pub fn flush(&mut self) {
        // Pad remaining bytes with 0s
        let buf_pos = self.write_pos % 8;
        if buf_pos == 0 {
            return;
        }
        self.cache[buf_pos..8].fill(0);
        self.write_cache(self.write_pos & !0x7);
    }

    fn write_cache(&mut self, offset: usize) {
        let word1 = u32::from_le_bytes(self.cache[0..4].try_into().unwrap());
        let word2 = u32::from_le_bytes(self.cache[4..8].try_into().unwrap());

        let dst1 = self.region.page_ptr(offset);
        let dst2 = self.region.page_ptr(offset + 4);

        unlock_flash();
        clear_errors();
        wait_busy();

        pac::FLASH.cr().modify(|w| w.set_pg(true));

        // Writing to flash must be done as a sequence of two 32-bit writes, starting on a 64-bit
        // aligned address
        fence(Ordering::SeqCst);
        unsafe { core::ptr::write_volatile(dst1, word1) };
        fence(Ordering::SeqCst);
        unsafe { core::ptr::write_volatile(dst2, word2) };
        fence(Ordering::SeqCst);
        wait_busy();

        pac::FLASH.sr().write(|w| w.set_eop(true));
        pac::FLASH.cr().modify(|w| w.set_pg(false));
        lock_flash();
    }
}

pub struct Stm32gxPagePair {
    page_a: Region,
    page_b: Region,
    write_region: Option<WriteRegion>,
}

impl Stm32gxPagePair {
    pub fn new(start_a: usize, start_b: usize, size: usize) -> Self {
        Self {
            page_a: Region {
                start_page: start_a,
                size,
            },
            page_b: Region {
                start_page: start_b,
                size,
            },
            write_region: None,
        }
    }

    pub fn lock(self) {
        lock_flash();
    }
}

impl Drop for Stm32gxPagePair {
    fn drop(&mut self) {
        lock_flash();
    }
}

impl DualPageFlash for Stm32gxPagePair {
    type Error = FlashError;

    fn page(&self, page: Page) -> &[u8] {
        match page {
            Page::A => self.page_a.page_slice(),
            Page::B => self.page_b.page_slice(),
        }
    }

    fn erase_page(&mut self, page: Page) {
        let region = match page {
            Page::A => self.page_a,
            Page::B => self.page_b,
        };
        self.write_region = Some(WriteRegion::new(region));
        self.write_region.as_mut().unwrap().erase();
    }

    fn write(&mut self, data: &[u8]) {
        let region = self.write_region.as_mut().unwrap();
        region.write(data);
    }

    fn flush(&mut self) {
        let region = self.write_region.as_mut().unwrap();
        region.flush();
    }
}
