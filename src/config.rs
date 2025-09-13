#![allow(unused_variables)]
#![allow(dead_code)]

use mipidsi::{models::ST7789, options::Rotation};
// DisplaySetting
pub const ROTATION: Rotation = Rotation::Deg270;
pub const MODEL: ST7789 = ST7789;

pub const LCD_WIDTH: usize = 320;
pub const LCD_HEIGHT: usize = 240;
pub const LCD_BUFFER_SIZE: usize = LCD_WIDTH * LCD_HEIGHT;