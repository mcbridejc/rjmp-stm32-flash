#![no_std]
#![no_main]

use rjmp_stm32_flash::{
    Stm32gxPagePair,
    dual_page::{DualPageFlash, Page},
};

use stm32_hal2 as hal;
use stm32_metapac as pac;

use cortex_m as _;
use defmt as _;
use panic_probe as _;

const REGION_A_START: usize = 60;
const REGION_B_START: usize = 62;
const REGION_SIZE: usize = 2; // number of pages

#[cortex_m_rt::entry]
fn main() -> ! {
    let channels = rtt_target::rtt_init! {
        up: {
            0: {
                size: 512,
                name: "defmt",
            }
        }
    };

    rtt_target::set_defmt_channel(channels.up.0);

    let clock_cfg = hal::clocks::Clocks::default();
    clock_cfg.setup().unwrap();

    // Enable clocks
    pac::RCC.apb1enr1().modify(|w| w.set_fdcanen(true));
    pac::RCC.apb1smenr1().modify(|w| w.set_fdcansmen(true));
    pac::RCC
        .ccipr()
        .modify(|w| w.set_fdcansel(stm32_metapac::rcc::vals::Fdcansel::PCLK1));
    pac::DBGMCU.cr().modify(|w| {
        w.set_dbg_standby(true);
        w.set_dbg_stop(true);
    });
    // Have to enable DMA1 clock to keep RAM accessible for RTT during debug
    pac::RCC.ahb1enr().modify(|w| w.set_dma1en(true));

    // Use last four flash pages as A and B storage pages
    let mut paged_flash = Stm32gxPagePair::new(REGION_A_START, REGION_B_START, REGION_SIZE);
    defmt::info!("Erasing...");
    paged_flash.erase_page(Page::A);
    defmt::info!("Writing part 1");
    paged_flash.write(&[0, 1, 2, 3]);
    defmt::info!("Writing part 2");
    paged_flash.write(&[4, 5, 6, 7, 8, 9]);
    defmt::info!("Flushing...");
    paged_flash.flush();

    defmt::info!("Complete!");
    let slice_a = &paged_flash.page(Page::A)[..10];
    defmt::info!("slice: 0x{:x}", slice_a.as_ptr());
    defmt::info!("slice_a len: {}", slice_a.len());
    defmt::info!("slice_a: {:?}", slice_a);
    if slice_a == &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
        defmt::info!("PAGE A Success");
    } else {
        defmt::info!("PAGE A FAILED: {:?}", slice_a);
    }

    loop {}
}
