use std::io::{self, Read};

use byteorder::{LittleEndian, ReadBytesExt};

pub const DICT_SIZE_MIN: u32 = 4096;
pub const DICT_SIZE_MAX: u32 = u32::MAX & !(15 as u32);

#[derive(Debug, Clone)]
pub struct LzmaHeaderProps {
    pub pb: u8,
    pub lp: u8,
    pub lc: u8,
}

#[derive(Debug, Clone)]
pub struct LzmaHeader {
    pub props: LzmaHeaderProps,
    pub dict_size: u32,
    pub uncompressed_size: u64,
}

fn parse_props_from_u8(props: u8) -> io::Result<LzmaHeaderProps> {
    if props > (4 * 5 + 4) * 9 + 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid LZMA properties byte",
        ));
    }

    let pb = props / (9 * 5);
    let props = props - pb * 9 * 5;
    let lp = props / 9;
    let lc = props - lp * 9;

    Ok(LzmaHeaderProps { pb, lp, lc })
}

pub fn parse_lzma_header(mut reader: impl Read) -> io::Result<LzmaHeader> {
    let props = parse_props_from_u8(reader.read_u8()?)?;
    let dict_size = reader.read_u32::<LittleEndian>()?;
    let uncompressed_size = reader.read_u64::<LittleEndian>()?;

    if dict_size > DICT_SIZE_MAX || dict_size < DICT_SIZE_MIN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid LZMA dictionary size",
        ));
    }

    Ok(LzmaHeader {
        props,
        dict_size,
        uncompressed_size,
    })
}
