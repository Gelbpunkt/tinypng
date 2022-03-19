/// This module contains a rather simple implementation of a PNG decoder.
/// To keep it simple, only "critical" chunks of the PNG spec have been
/// implemented.
/// Loosely based on https://www.w3.org/TR/2003/REC-PNG-20031110/
use std::{
    collections::VecDeque,
    io::{self, Read},
};

use flate2::{read::ZlibDecoder, Crc};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

const ZLIB_COMPRESSION_METHOD: u8 = 0;

// PLTE is required in indexed, allowed in truecolor and truecolor alpha and forbidden in grayscale and grayscale alpha
const _GRAYSCALE: u8 = 0;
const TRUECOLOR: u8 = 2;
const _INDEXED_COLOR: u8 = 3;
const _GRAYSCALE_ALPHA: u8 = 4;
const TRUECOLOR_ALPHA: u8 = 6;

const FILTER_NONE: u8 = 0;
const FILTER_SUB: u8 = 1;
const FILTER_UP: u8 = 2;
const FILTER_AVG: u8 = 3;
const FILTER_PAETH: u8 = 4;

fn recon_a(recon: &[u8], stride: u32, bytes_per_pixel: u32, r: u32, c: u32) -> u8 {
    if c >= bytes_per_pixel {
        recon[(r * stride + c - bytes_per_pixel) as usize]
    } else {
        0
    }
}

fn recon_b(recon: &[u8], stride: u32, r: u32, c: u32) -> u8 {
    if r > 0 {
        recon[((r - 1) * stride + c) as usize]
    } else {
        0
    }
}

fn recon_c(recon: &[u8], stride: u32, bytes_per_pixel: u32, r: u32, c: u32) -> u8 {
    if r > 0 && c >= bytes_per_pixel {
        recon[((r - 1) * stride + c - bytes_per_pixel) as usize]
    } else {
        0
    }
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let p = a + b - c;
    let pa = p.abs_diff(a);
    let pb = p.abs_diff(b);
    let pc = p.abs_diff(c);

    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[derive(Debug)]
pub enum PixelType {
    Rgb,
    Rgba,
}

impl PixelType {
    fn bytes(&self) -> u32 {
        match self {
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

#[derive(Debug)]
pub enum Pixel {
    Rgb([u8; 3]),
    Rgba([u8; 4]),
}

impl Pixel {
    pub fn raw(&self) -> &[u8] {
        match self {
            Self::Rgb(data) => data,
            Self::Rgba(data) => data,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    InvalidSignature,
    InvalidStartingChunk,
    Unimplemented,
    InvalidIHDRLength,
    InvalidPLTESize,
    UnsupportedCompressionMethod,
    InvalidFilterType,
    MismatchedCrc,
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

#[derive(Debug)]
enum Chunk {
    Ihdr(Ihdr),
    Plte(Plte),
    Idat(Vec<u8>),
    Iend,
}

impl Chunk {
    fn from_raw(type_bytes: [u8; 4], data: Vec<u8>) -> Result<Option<Self>, Error> {
        // Lowercase-starting chunks are optional, and therefore we don't have to fail
        // with unimplemented
        let is_optional = type_bytes[0] >= b'a' && type_bytes[0] <= b'z';

        match &type_bytes {
            b"IHDR" => Ok(Some(Self::Ihdr(Ihdr::from_data(&data)?))),
            b"PLTE" => Ok(Some(Self::Plte(Plte::from_data(&data)?))),
            b"IDAT" => Ok(Some(Self::Idat(data))),
            b"IEND" => Ok(Some(Self::Iend)),
            _ => {
                if is_optional {
                    Ok(None)
                } else {
                    Err(Error::Unimplemented)
                }
            }
        }
    }

    fn read<T: Read>(stream: &mut T) -> Result<Option<Self>, Error> {
        let mut length_bytes = [0; 4];
        stream.read_exact(&mut length_bytes)?;
        let length = u32::from_be_bytes(length_bytes[..4].try_into().unwrap());

        let mut r#type = [0; 4];
        stream.read_exact(&mut r#type)?;

        let mut data = vec![0; length as usize];
        stream.read_exact(&mut data)?;

        let mut crc_bytes = [0; 4];
        stream.read_exact(&mut crc_bytes)?;

        let crc = u32::from_be_bytes(crc_bytes);

        let mut hasher = Crc::new();
        hasher.update(&r#type);
        hasher.update(&data);

        if crc == hasher.sum() {
            Self::from_raw(r#type, data)
        } else {
            Err(Error::MismatchedCrc)
        }
    }
}

#[derive(Debug)]
struct Ihdr {
    width: u32,
    height: u32,
    bit_depth: u8,
    colour_type: u8,
    compression_method: u8,
    filter_method: u8,
    interlace_method: u8,
}

impl Ihdr {
    fn from_data(data: &[u8]) -> Result<Self, Error> {
        if data.len() != 13 {
            return Err(Error::InvalidIHDRLength);
        }

        let width = u32::from_be_bytes(data[..4].try_into().unwrap());
        let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
        let bit_depth = data[8];
        let colour_type = data[9];
        let compression_method = data[10];
        let filter_method = data[11];
        let interlace_method = data[12];

        Ok(Self {
            width,
            height,
            bit_depth,
            colour_type,
            compression_method,
            filter_method,
            interlace_method,
        })
    }

    fn pixel_type(&self) -> Result<PixelType, Error> {
        match self.colour_type {
            TRUECOLOR => Ok(PixelType::Rgb),
            TRUECOLOR_ALPHA => Ok(PixelType::Rgba),
            _ => Err(Error::Unimplemented),
        }
    }
}

#[derive(Debug)]
struct Plte {
    entries: Vec<Pixel>,
}

impl Plte {
    fn from_data(data: &[u8]) -> Result<Self, Error> {
        let entry_count = data.len() / 3;

        if data.len() % 3 != 0 || entry_count < 1 || entry_count > 256 {
            return Err(Error::InvalidPLTESize);
        }

        let mut entries = Vec::with_capacity(entry_count);

        for i in 0..entry_count {
            entries.push(Pixel::Rgb([data[i * 3], data[i * 3 + 1], data[i * 3 + 2]]));
        }

        Ok(Self { entries })
    }
}

#[derive(Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixel_type: PixelType,
    pub pixels: Vec<Vec<Pixel>>,
}

impl Image {
    fn from_chunks(mut chunks: VecDeque<Chunk>) -> Result<Self, Error> {
        let ihdr = match chunks.pop_front() {
            Some(Chunk::Ihdr(ihdr)) => ihdr,
            _ => return Err(Error::InvalidStartingChunk),
        };

        let plte = if matches!(chunks.front(), Some(Chunk::Plte(_))) {
            if let Some(Chunk::Plte(plte)) = chunks.pop_front() {
                Some(plte)
            } else {
                None
            }
        } else {
            None
        };

        let idat_data_compressed: Vec<u8> = chunks
            .into_iter()
            .filter_map(|chunk| {
                if let Chunk::Idat(data) = chunk {
                    Some(data)
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        let pixel_type = ihdr.pixel_type()?;
        let bytes_per_pixel = pixel_type.bytes();

        let expected_idat_data_length = ihdr.height * (1 + ihdr.width * bytes_per_pixel);
        let mut idat_data = Vec::with_capacity(expected_idat_data_length as usize);

        if ihdr.compression_method == ZLIB_COMPRESSION_METHOD {
            let mut decoder = ZlibDecoder::new(&idat_data_compressed[..]);
            decoder.read_to_end(&mut idat_data)?;
        } else {
            return Err(Error::UnsupportedCompressionMethod);
        };

        let stride = ihdr.width * bytes_per_pixel;
        let mut recon = Vec::with_capacity((ihdr.height * stride) as usize);

        let mut i = 0;
        for r in 0..ihdr.height {
            let filter_type = idat_data[i];
            i += 1;

            for c in 0..stride {
                let byte = idat_data[i];
                i += 1;

                let recon_byte = match filter_type {
                    FILTER_NONE => byte,
                    FILTER_SUB => byte + recon_a(&recon, stride, bytes_per_pixel, r, c),
                    FILTER_UP => byte + recon_b(&recon, stride, r, c),
                    FILTER_AVG => {
                        byte + (recon_a(&recon, stride, bytes_per_pixel, r, c)
                            + recon_b(&recon, stride, r, c))
                            / 2
                    }
                    FILTER_PAETH => {
                        byte + paeth_predictor(
                            recon_a(&recon, stride, bytes_per_pixel, r, c),
                            recon_b(&recon, stride, r, c),
                            recon_c(&recon, stride, bytes_per_pixel, r, c),
                        )
                    }
                    _ => return Err(Error::InvalidFilterType),
                };

                recon.push(recon_byte);
            }
        }

        let mut pixels = Vec::with_capacity(ihdr.height as usize);

        let bytes_per_row = ihdr.width * bytes_per_pixel;

        for y in 0..ihdr.height {
            let mut row = Vec::with_capacity(ihdr.width as usize);

            for x in 0..ihdr.width {
                let idx = (y * bytes_per_row + x * bytes_per_pixel) as usize;

                let pixel = match pixel_type {
                    PixelType::Rgb => Pixel::Rgb([recon[idx], recon[idx + 1], recon[idx + 2]]),
                    PixelType::Rgba => {
                        Pixel::Rgba([recon[idx], recon[idx + 1], recon[idx + 2], recon[idx + 3]])
                    }
                };

                row.push(pixel);
            }

            pixels.push(row);
        }

        Ok(Self {
            width: ihdr.width,
            height: ihdr.height,
            pixel_type,
            pixels,
        })
    }

    pub fn read<T: Read>(stream: &mut T) -> Result<Self, Error> {
        let mut signature = [0; 8];
        stream.read_exact(&mut signature)?;

        if signature != PNG_SIGNATURE {
            return Err(Error::InvalidSignature);
        }

        let mut chunks = VecDeque::new();

        loop {
            if let Some(chunk) = Chunk::read(stream)? {
                let is_end = matches!(chunk, Chunk::Iend);
                chunks.push_back(chunk);

                if is_end {
                    break;
                }
            }
        }

        Self::from_chunks(chunks)
    }
}
