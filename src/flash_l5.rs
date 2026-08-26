//! Flash driver for the non-secure flash interface on STM32L5 devices.
//!
//! The STM32L552RC is 256 KiB and uses 128 2 KiB pages in its default dual-bank
//! configuration. Page numbers 0-63 select a page within a bank and NSBKER
//! selects the bank. Programming is performed one 64-bit double word at a time
//! through the non-secure FLASH registers.

use core::{
    ptr::slice_from_raw_parts,
    sync::atomic::{Ordering, fence},
};

use embedded_storage::nor_flash::{ErrorType, NorFlashError, NorFlashErrorKind, ReadNorFlash};
use stm32_metapac as pac;

use crate::dual_page::{DualPageFlash, Page};

pub const STM32L552_PAGE_SIZE: usize = 2048;
const FLASH_BASE: usize = 0x0800_0000;
const PAGES_PER_BANK: usize = 64;
const NUM_PAGES: usize = PAGES_PER_BANK * 2;
const FLASH_SIZE: usize = STM32L552_PAGE_SIZE * NUM_PAGES;
const WRITE_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlashError {
    OutOfRange,
}

impl NorFlashError for FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::OutOfRange => NorFlashErrorKind::OutOfBounds,
        }
    }
}

pub struct Stm32l5Flash;

impl ErrorType for Stm32l5Flash {
    type Error = FlashError;
}

impl Stm32l5Flash {
    pub const fn new() -> Self {
        Self
    }

    pub fn in_range(offset: usize, size: usize) -> bool {
        offset
            .checked_add(size)
            .is_some_and(|end| end <= FLASH_SIZE)
    }
}

impl Default for Stm32l5Flash {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadNorFlash for Stm32l5Flash {
    const READ_SIZE: usize = 1;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let offset = offset as usize;
        if !Self::in_range(offset, bytes.len()) {
            return Err(FlashError::OutOfRange);
        }

        let data =
            unsafe { core::slice::from_raw_parts((FLASH_BASE + offset) as *const u8, bytes.len()) };
        bytes.copy_from_slice(data);
        Ok(())
    }

    fn capacity(&self) -> usize {
        FLASH_SIZE
    }
}

fn unlock_flash() {
    if pac::FLASH.nscr().read().nslock() {
        pac::FLASH.nskeyr().write_value(0x4567_0123);
        pac::FLASH.nskeyr().write_value(0xCDEF_89AB);
    }
}

fn lock_flash() {
    wait_busy();
    pac::FLASH.nscr().modify(|w| w.set_nslock(true));
}

fn wait_busy() {
    while pac::FLASH.nssr().read().nsbsy() {}
}

fn clear_errors() {
    pac::FLASH.nssr().write(|w| {
        w.set_nsoperr(true);
        w.set_nsprogerr(true);
        w.set_nswrperr(true);
        w.set_nspgaerr(true);
        w.set_nssizerr(true);
        w.set_nspgserr(true);
        w.set_optwerr(true);
    });
}

fn erase_physical_page(page_num: usize) {
    assert!(page_num < NUM_PAGES);
    unlock_flash();
    wait_busy();
    clear_errors();

    pac::FLASH.nscr().modify(|w| {
        w.set_nsbker(page_num >= PAGES_PER_BANK);
        w.set_nspnb((page_num % PAGES_PER_BANK) as u8);
        w.set_nsper(true);
    });
    pac::FLASH.nscr().modify(|w| w.set_nsstrt(true));
    wait_busy();
    pac::FLASH.nscr().modify(|w| w.set_nsper(false));
    pac::FLASH.nssr().write(|w| w.set_nseop(true));
    lock_flash();
}

#[derive(Clone, Copy, Debug)]
struct Region {
    start_page: usize,
    size: usize,
}

impl Region {
    fn as_slice(&self) -> &'static [u8] {
        let addr = (FLASH_BASE + self.start_page * STM32L552_PAGE_SIZE) as *const u8;
        unsafe {
            slice_from_raw_parts(addr, STM32L552_PAGE_SIZE * self.size)
                .as_ref()
                .unwrap()
        }
    }

    fn ptr(&self, offset: usize) -> *mut u32 {
        (FLASH_BASE + self.start_page * STM32L552_PAGE_SIZE + offset) as *mut u32
    }
}

struct WriteRegion {
    region: Region,
    write_pos: usize,
    cache: [u8; WRITE_SIZE],
}

impl WriteRegion {
    fn new(region: Region) -> Self {
        Self {
            region,
            write_pos: 0,
            cache: [0xff; WRITE_SIZE],
        }
    }

    fn erase(&mut self) {
        for page in self.region.start_page..self.region.start_page + self.region.size {
            erase_physical_page(page);
        }
    }

    fn write(&mut self, data: &[u8]) {
        assert!(self.write_pos + data.len() <= self.region.as_slice().len());
        let mut input_pos = 0;
        while input_pos < data.len() {
            let cache_pos = self.write_pos % WRITE_SIZE;
            let count = (WRITE_SIZE - cache_pos).min(data.len() - input_pos);
            self.cache[cache_pos..cache_pos + count]
                .copy_from_slice(&data[input_pos..input_pos + count]);
            input_pos += count;
            self.write_pos += count;
            if self.write_pos % WRITE_SIZE == 0 {
                self.write_cache(self.write_pos - WRITE_SIZE);
            }
        }
    }

    fn flush(&mut self) {
        let cache_pos = self.write_pos % WRITE_SIZE;
        if cache_pos != 0 {
            self.cache[cache_pos..].fill(0xff);
            self.write_cache(self.write_pos & !(WRITE_SIZE - 1));
        }
    }

    fn write_cache(&mut self, offset: usize) {
        let word1 = u32::from_le_bytes(self.cache[..4].try_into().unwrap());
        let word2 = u32::from_le_bytes(self.cache[4..].try_into().unwrap());
        let dst1 = self.region.ptr(offset);
        let dst2 = self.region.ptr(offset + 4);

        unlock_flash();
        wait_busy();
        clear_errors();

        cortex_m::interrupt::free(|_| {
            pac::FLASH.nscr().modify(|w| w.set_nspg(true));
            fence(Ordering::SeqCst);
            unsafe { core::ptr::write_volatile(dst1, word1) };
            fence(Ordering::SeqCst);
            unsafe { core::ptr::write_volatile(dst2, word2) };
            fence(Ordering::SeqCst);
            wait_busy();
            pac::FLASH.nscr().modify(|w| w.set_nspg(false));
        });

        pac::FLASH.nssr().write(|w| w.set_nseop(true));
        lock_flash();
        self.cache.fill(0xff);
    }
}

pub struct Stm32l5PagePair {
    page_a: Region,
    page_b: Region,
    write_region: Option<WriteRegion>,
}

impl Stm32l5PagePair {
    /// create a new page pair
    ///
    /// # Arguments
    /// start_a: The starting page number for first flash section
    /// start_b: The starting page number for second flash section
    /// size: The number of pages in each section
    pub fn new(start_a: usize, start_b: usize, size: usize) -> Self {
        assert!(
            pac::FLASH.optr().read().dbank(),
            "Stm32l5PagePair requires dual-bank mode"
        );
        assert!(size > 0);
        assert!(start_a + size <= NUM_PAGES);
        assert!(start_b + size <= NUM_PAGES);
        assert!(start_a + size <= start_b || start_b + size <= start_a);
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
}

impl Drop for Stm32l5PagePair {
    fn drop(&mut self) {
        lock_flash();
    }
}

impl DualPageFlash for Stm32l5PagePair {
    type Error = FlashError;

    fn page(&self, page: Page) -> &[u8] {
        match page {
            Page::A => self.page_a.as_slice(),
            Page::B => self.page_b.as_slice(),
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
        self.write_region.as_mut().unwrap().write(data);
    }

    fn flush(&mut self) {
        self.write_region.as_mut().unwrap().flush();
    }
}
