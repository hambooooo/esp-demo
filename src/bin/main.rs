#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;
use esp_demo::config;
use esp_demo::raw_framebuffer;

use core::ptr::addr_of_mut;

use alloc::boxed::Box;
use alloc::format;
use bevy_ecs::prelude::*;
use bevy_ecs::{schedule::Schedule, world::World};
use config::{LCD_HEIGHT, LCD_WIDTH};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::mono_font::ascii::FONT_8X13;
use embedded_graphics::prelude::Size;
use embedded_graphics::primitives::StyledDrawable;
use embedded_graphics::Drawable;
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::{Point, RgbColor},
    primitives::Rectangle,
    text::Text,
};
use embedded_hal::delay::DelayNs;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::system::{CpuControl, Stack};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::AnyTimer;
use esp_hal::{
    delay::Delay,
    dma::{DmaRxBuf, DmaTxBuf},
    dma_buffers,
    gpio::{Level, Output, OutputConfig},
    spi::master::{Spi, SpiDmaBus},
    time::Rate,
    Blocking,
};
use esp_hal_embassy::Executor;
use esp_println::logger::init_logger_from_env;
use mipidsi::{interface::SpiInterface, models::ST7789, options::Orientation, Builder};
use raw_framebuffer::RawFramebuffer;
use static_cell::StaticCell;

static mut APP_CORE_STACK: Stack<1024> = Stack::new();

static MOSI: Channel<CriticalSectionRawMutex, &mut RawFramebuffer<Rgb565>, 2> = Channel::new();
static MISO: Channel<CriticalSectionRawMutex, &mut RawFramebuffer<Rgb565>, 2> = Channel::new();

struct DisplayResource {
    display: mipidsi::Display<
        SpiInterface<
            'static,
            ExclusiveDevice<SpiDmaBus<'static, Blocking>, Output<'static>, Delay>,
            Output<'static>,
        >,
        ST7789,
        Output<'static>,
    >,
}

#[embassy_executor::task]
async fn run1(mut display_res: DisplayResource) {
    let area = Rectangle::new(
        Point::zero(),
        Size {
            width: config::LCD_WIDTH as u32,
            height: config::LCD_HEIGHT as u32,
        },
    );
    loop {
        // Flush the framebuffer to the physical display.
        let fb = MOSI.receive().await;
        display_res
            .display
            .fill_contiguous(&area, fb.data.iter().copied())
            .unwrap();
        MISO.try_send(fb).expect("MISO send framebuffer error");
    }
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(size: 158*1024);
    init_logger_from_env();
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let timer0: AnyTimer = timg0.timer0.into();
    let timer1: AnyTimer = timg0.timer1.into();
    esp_hal_embassy::init([timer0, timer1]);
    let mut cpu_control = CpuControl::new(peripherals.CPU_CTRL);
    log::info!("Embassy initialized!");

    // --- DMA Buffers for SPI ---
    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = dma_buffers!(1, 2048);
    let dma_rx_buf = DmaRxBuf::new(rx_descriptors, rx_buffer).unwrap();
    let dma_tx_buf = DmaTxBuf::new(tx_descriptors, tx_buffer).unwrap();

    // --- Display Setup using BSP values ---
    let spi = Spi::<Blocking>::new(
        peripherals.SPI3,
        esp_hal::spi::master::Config::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(esp_hal::spi::Mode::_3),
    )
    .unwrap()
    .with_sck(peripherals.GPIO18)
    .with_mosi(peripherals.GPIO21)
    .with_dma(peripherals.DMA_CH3)
    .with_buffers(dma_rx_buf, dma_tx_buf);
    let cs_output = Output::new(peripherals.GPIO5, Level::High, OutputConfig::default());
    let spi_delay = Delay::new();
    let spi_device = ExclusiveDevice::new(spi, cs_output, spi_delay).unwrap();

    // LCD interface
    let lcd_dc = Output::new(peripherals.GPIO16, Level::Low, OutputConfig::default());
    // Leak a Box to obtain a 'static mutable buffer.
    let buffer: &'static mut [u8; 2048] = Box::leak(Box::new([0_u8; 2048]));
    let di = SpiInterface::new(spi_device, lcd_dc, buffer);

    let mut display_delay = Delay::new();
    display_delay.delay_ns(500_000u32);

    // Reset pin: OpenDrain required for ESP32-S3-BOX! Tricky setting.
    // For some Wrover-Kit boards the reset pin must be pulsed low.
    let mut reset = Output::new(peripherals.GPIO17, Level::Low, OutputConfig::default());
    // Pulse the reset pin: drive low for 100 ms then high.
    reset.set_low();
    Delay::new().delay_ms(100u32);
    reset.set_high();

    // Initialize the display using mipidsi's builder.
    let mut display = Builder::new(config::MODEL, di)
        .reset_pin(reset)
        .color_order(mipidsi::options::ColorOrder::Bgr)
        .orientation(Orientation::default().rotate(config::ROTATION))
        .init(&mut display_delay)
        .unwrap();

    display.clear(Rgb565::BLACK).unwrap();

    let mut world = World::default();
    let mut schedule = Schedule::default();
    schedule.add_systems(render_system);

    log::info!("data0");
    let data0 = Box::new([Rgb565::BLACK; config::LCD_BUFFER_SIZE]);
    log::info!("fb0");
    let fb0 = Box::leak(Box::new(RawFramebuffer::new(
        Box::leak(data0),
        LCD_WIDTH as u32,
        LCD_HEIGHT as u32,
    )));
    log::info!("data1");
    let data1 = Box::new([Rgb565::BLACK; config::LCD_BUFFER_SIZE]);
    let fb1 = Box::leak(Box::new(RawFramebuffer::new(
        Box::leak(data1),
        LCD_WIDTH as u32,
        LCD_HEIGHT as u32,
    )));

    log::info!("MOSI");
    MOSI.send(fb0).await;
    MOSI.send(fb1).await;

    log::info!("spawner");
    // let r: embassy_sync::channel::Receiver<
    //     '_,
    //     CriticalSectionRawMutex,
    //     &mut RawFramebuffer<'_, Rgb565>,
    //     2,
    // > = mosi.receiver();

    let _guard = cpu_control
        .start_app_core(unsafe { &mut *addr_of_mut!(APP_CORE_STACK) }, move || {
            static EXECUTOR: StaticCell<Executor> = StaticCell::new();
            let executor = EXECUTOR.init(Executor::new());
            executor.run(|spawner| {
                spawner.spawn(run1(DisplayResource { display })).ok();
            });
        })
        .unwrap();

    let _spawner = spawner;
    log::info!("schedule");
    loop {
        schedule.run(&mut world);
    }
}

fn render_system(mut prev_time: Local<u64>, mut prev_update_time: Local<u64>) {
    let time = embassy_time::Instant::now().as_millis();
    let delta = time - *prev_update_time;
    let fps = if delta == 0 { 0 } else { 1000 / delta };
    let sps = time - *prev_time;
    *prev_time = time;

    let Ok(fb) = MISO.try_receive() else {
        return;
    };
    *prev_update_time = time;

    // Clear the framebuffer.
    fb.clear(Rgb565::BLUE).unwrap();

    Rectangle::new(Point::new(10, 10), Size::new(200, 156))
        .draw_styled(
            &embedded_graphics::primitives::PrimitiveStyle::with_fill(Rgb565::BLACK),
            fb,
        )
        .unwrap();
    Text::new(
        &format!("FPS:{}, sps:{}", fps, sps),
        Point::new(20, 20),
        MonoTextStyle::new(&FONT_8X13, Rgb565::WHITE),
    )
    .draw(fb)
    .unwrap();

    MOSI.try_send(fb).expect("MOSI send framebuffer error");
}
