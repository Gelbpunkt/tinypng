use show_image::{create_window, ImageInfo, ImageView};
use tinypng::png::{Image, PixelType};

use std::{io::Cursor, time::Instant};

static INPUT_FILE: &[u8] = include_bytes!("../files/idlerpg.png");

#[show_image::main]
fn test_png_decode() {
    let mut cursor = Cursor::new(INPUT_FILE);

    let before = Instant::now();
    let img = Image::read(&mut cursor).unwrap();
    let after = Instant::now();

    let title = format!("Decoding PNG took {:?}", after - before);

    let info = match img.pixel_type {
        PixelType::Rgb => ImageInfo::rgb8(img.width, img.height),
        PixelType::Rgba => ImageInfo::rgba8(img.width, img.height),
    };
    let mut data = Vec::new();

    for row in img.pixels {
        for pixel in row {
            data.extend_from_slice(pixel.raw());
        }
    }

    let image = ImageView::new(info, &data);

    // Create a window with default options and display the image.
    let window = create_window(title, Default::default()).unwrap();
    window.set_image("image-001", image).unwrap();
    window.wait_until_destroyed().unwrap();
}
