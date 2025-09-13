use embedded_graphics::{
    Pixel,
    prelude::{Dimensions, DrawTarget, Point, RgbColor, Size},
    primitives::Rectangle,
};

use crate::config::LCD_BUFFER_SIZE;

#[derive(Debug)]
pub struct RawFramebuffer<'a, C: RgbColor> {
    pub data: &'a mut [C; LCD_BUFFER_SIZE],
    width: u32,
    height: u32,
}

impl<'a, C> RawFramebuffer<'a, C>
where
    C: RgbColor,
{
    pub fn new(data: &'a mut [C; LCD_BUFFER_SIZE], width: u32, height: u32) -> Self {
        RawFramebuffer {
            data,
            width,
            height,
        }
    }
}

impl<'a, C> Dimensions for RawFramebuffer<'a, C>
where
    C: RgbColor,
{
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::new(0, 0), Size::new(self.width, self.height))
    }
}

impl<'a, C> DrawTarget for RawFramebuffer<'a, C>
where
    C: RgbColor,
{
    type Color = C;
    type Error = ();

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for pixel in pixels {
            let x = pixel.0.x as usize;
            let y = pixel.0.y as usize;
            if x < self.width as usize
                && y < self.height as usize
                && y * self.width as usize + x < self.data.len()
            {
                self.data[y * self.width as usize + x] = pixel.1;
            }
        }
        Ok(())
    }
}