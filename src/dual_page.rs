//! Trait for abstracting dual page storage
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Page {
    A,
    B,
}

impl Page {
    pub fn other(&self) -> Self {
        match self {
            Page::A => Page::B,
            Page::B => Page::A,
        }
    }
}

/// A trait to be implemented by a flash driver to allow writing persistent data
pub trait DualPageFlash {
    type Error;

    /// Get one of the pages as a slice
    fn page(&self, page: Page) -> &[u8];

    /// Select one of the pages for writing and erase it
    fn erase_page(&mut self, page: Page);

    /// Write data to the already erased page
    ///
    /// Data can be written in any sizes, but due to restrictions in the write word size, may be
    /// cached until a complete word is available. Call flush after all data is written to ensure
    /// the last word is written to flash.
    ///
    /// `erase_page()` must be called before calling write, or it will panic.
    fn write(&mut self, data: &[u8]);

    /// Must be called after completing all writes
    fn flush(&mut self);
}
